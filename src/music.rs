use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::package::{Export, Package};

pub struct Track {
    pub name: String,
    pub module: Vec<u8>,
    pub directory: PathBuf,
}

pub fn load(map: &Package<'_>, actors: &[&Export], path: &Path) -> Option<Track> {
    let reference = actors
        .iter()
        .filter(|actor| map.object_name(actor.class) == Some("LevelInfo"))
        .find_map(|actor| {
            let mut props = map.properties(actor).ok()?;
            while let Some((name, mut data)) = props.next().ok()? {
                if name == "Song" {
                    return data.compact().ok();
                }
            }
            None
        })?;
    let (package_name, _) = crate::texture::import_path(map, reference)?;
    let directory = path.parent()?.parent()?;
    let asset = fs::read_dir(directory.join("Music"))
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| {
            let path = entry.path();
            path.extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("umx"))
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case(&package_name))
        })?;
    let bytes = fs::read(asset.path()).ok()?;
    let package = Package::read(&bytes).ok()?;
    let export = package
        .exports
        .iter()
        .find(|entry| package.object_name(entry.class) == Some("Music"))?;
    let mut reader = crate::map::native_data(&package, export).ok()?;
    reader.compact().ok()?; // Tracker format name.
    if package.version > 61 {
        reader.u32().ok()?;
    }
    let length = reader.count(1).ok()?;
    Some(Track {
        name: package.names[export.name].clone(),
        module: reader.take(length).ok()?.to_vec(),
        directory: directory.to_path_buf(),
    })
}

#[cfg(windows)]
pub mod player {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr::null_mut};
    use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        LoadLibraryExW,
    };

    use super::Track;

    type Module = *mut c_void;
    type Create = unsafe extern "C" fn(
        *const c_void,
        usize,
        Option<unsafe extern "C" fn(*const i8, *mut c_void)>,
        *mut c_void,
        Option<unsafe extern "C" fn(i32, *mut c_void) -> i32>,
        *mut c_void,
        *mut i32,
        *mut *const i8,
        *const c_void,
    ) -> Module;
    type Destroy = unsafe extern "C" fn(Module);
    type Read = unsafe extern "C" fn(Module, i32, usize, *mut i16) -> usize;
    type Repeat = unsafe extern "C" fn(Module, i32) -> i32;

    struct Library(HMODULE);

    impl Drop for Library {
        fn drop(&mut self) {
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }

    pub struct Music {
        module: Module,
        read: Read,
        destroy: Destroy,
        _library: Library,
    }

    impl Music {
        pub fn open(track: &Track) -> Option<Self> {
            let library_path = track.directory.join("libopenmpt.dll").canonicalize().ok()?;
            let name: Vec<_> = library_path.as_os_str().encode_wide().chain([0]).collect();
            let handle = unsafe {
                LoadLibraryExW(
                    name.as_ptr(),
                    null_mut(),
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
                )
            };
            if handle.is_null() {
                return None;
            }
            let library = Library(handle);
            // The signatures are from libopenmpt's public C ABI. The module is
            // destroyed before the DLL is released.
            unsafe {
                let create: Create = std::mem::transmute(GetProcAddress(
                    handle,
                    c"openmpt_module_create_from_memory2".as_ptr().cast(),
                )?);
                let read: Read = std::mem::transmute(GetProcAddress(
                    handle,
                    c"openmpt_module_read_interleaved_stereo".as_ptr().cast(),
                )?);
                let destroy: Destroy = std::mem::transmute(GetProcAddress(
                    handle,
                    c"openmpt_module_destroy".as_ptr().cast(),
                )?);
                let repeat: Repeat = std::mem::transmute(GetProcAddress(
                    handle,
                    c"openmpt_module_set_repeat_count".as_ptr().cast(),
                )?);
                let module = create(
                    track.module.as_ptr().cast(),
                    track.module.len(),
                    None,
                    null_mut(),
                    None,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    std::ptr::null(),
                );
                if module.is_null() {
                    return None;
                }
                repeat(module, -1);
                Some(Self {
                    module,
                    read,
                    destroy,
                    _library: library,
                })
            }
        }

        pub fn render(&mut self, rate: u32, out: &mut [i16]) -> usize {
            unsafe { (self.read)(self.module, rate as i32, out.len() / 2, out.as_mut_ptr()) * 2 }
        }
    }

    impl Drop for Music {
        fn drop(&mut self) {
            unsafe {
                (self.destroy)(self.module);
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::map::Map;

    #[test]
    #[ignore = "requires the local GOTY soundtrack and libopenmpt DLL"]
    fn deck16_music_decodes_audible_stereo() {
        let path = Path::new("local/goty/Maps/DM-Deck16][.unr");
        let data = fs::read(path).unwrap();
        let package = Package::read(&data).unwrap();
        let map = Map::load(&package, Some(path)).unwrap();
        let track = map.music.as_ref().expect("Deck16 music reference and UMX");
        assert!(track.name.to_ascii_lowercase().contains("godown"));
        let mut music = player::Music::open(track).expect("OpenMPT decoder");
        let mut samples = [0; 8192];
        let mut audible = false;
        for _ in 0..20 {
            let count = music.render(44_100, &mut samples);
            assert_eq!(count, samples.len());
            audible |= samples
                .as_chunks::<2>()
                .0
                .iter()
                .any(|frame| frame[0] != frame[1] && frame[0].abs() > 25);
        }
        assert!(audible);
    }
}
