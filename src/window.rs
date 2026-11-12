use std::io;
use std::path::Path;
use std::ptr::null_mut;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Input::{
    GetRawInputData, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::collision::{MotionEvents, Walker};
use crate::frontend::{Action, Frontend, Menu};
use crate::game::{Input, Practice};
use crate::hud::Hud;
use crate::map::{Map, Vec3};
use crate::present::Presenter;
use crate::render::{Camera, Frame};
use crate::settings::{RESOLUTIONS, Settings};
use crate::sound::device::Audio;

pub fn show(
    map: &mut Map,
    start: usize,
    title: &str,
    obs: bool,
    path: &Path,
    startup: bool,
) -> io::Result<()> {
    let settings_path = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("../UT99-Rust.ini");
    let mut settings = Settings::load(&settings_path)?;
    let mut frontend = startup.then(|| Frontend::load(path, settings.intro));
    let class: Vec<u16> = "Ut99RustViewer\0".encode_utf16().collect();
    let title: Vec<u16> = title.encode_utf16().chain([0]).collect();

    // The HWND, its device contexts and the message pump all live on this thread.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(std::ptr::null());
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        if RegisterClassW(&window_class) == 0 {
            return Err(io::Error::last_os_error());
        }
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1120,
            680,
            null_mut(),
            null_mut(),
            instance,
            std::ptr::null(),
        );
        if window.is_null() {
            return Err(io::Error::last_os_error());
        }
        message_loop(
            window,
            map,
            start,
            obs,
            frontend.as_mut(),
            &mut settings,
            &settings_path,
        )
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            WM_ERASEBKGND => 1,
            WM_DPICHANGED
                if GetWindowLongW(window, GWL_STYLE) as u32 & WS_OVERLAPPEDWINDOW != 0 =>
            {
                let rect = &*(lparam as *const RECT);
                SetWindowPos(
                    window,
                    null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

unsafe fn message_loop(
    window: HWND,
    map: &mut Map,
    mut start: usize,
    obs: bool,
    mut frontend: Option<&mut Frontend>,
    settings: &mut Settings,
    settings_path: &Path,
) -> io::Result<()> {
    let mut frame = Frame::hardware(map);
    let hud = Hud::new();
    let mut game = Practice::new(map);
    let mut mouse = Mouse::new(window);
    let mut music_muted = false;
    let mut primary = false;
    let mut alternate = false;
    let mut slot = None;
    let mut cycle = 0;
    let mut camera = Camera::from_start(map.starts[start]);
    let mut walker = Walker::spawn(&map.collision, map.starts[start].position);
    let mut fly = false;
    let mut muted = false;
    let mut audio = Audio::open(if frontend.is_some() {
        None
    } else {
        map.music.as_ref()
    });
    let mut jump = false;
    let mut occupied_portal = None;
    let mut last_direction = None;
    let mut dodge = None;
    let kill_z = map
        .starts
        .iter()
        .map(|start| start.position.2)
        .fold(f32::INFINITY, f32::min)
        - 4096.0;
    let mut presenter = None;
    let mut windowed = None;
    let mut pilot_view = None;
    let mut obs_announced = false;
    set_window_size(window, settings.resolution);
    if obs || settings.fullscreen {
        toggle_borderless(window, &mut windowed);
    }
    unsafe {
        ShowWindow(window, SW_SHOW);
    }
    if let Some(frontend) = frontend.as_deref_mut() {
        if !run_frontend(
            window,
            map,
            frontend,
            &hud,
            &mut presenter,
            &mut audio,
            settings,
            settings_path,
            &mut windowed,
            obs,
            false,
        )? {
            return Ok(());
        }
        if let Some(audio) = &mut audio {
            audio.set_track(map.music.as_ref());
        }
        if unsafe { GetForegroundWindow() == window } {
            mouse.capture();
        }
    }
    let started = Instant::now();
    let mut tick = started;
    let mut stats = FrameStats::new(started);

    unsafe {
        loop {
            let mut raw = (0i32, 0i32);
            let mut message = MSG::default();
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    return Ok(());
                }
                if message.hwnd == window {
                    let can_fire = mouse.captured || game.death.is_some();
                    match message.message {
                        WM_KILLFOCUS => mouse.release(),
                        WM_SIZE if message.wParam == SIZE_MINIMIZED as usize => mouse.release(),
                        WM_SIZE | WM_MOVE if mouse.captured => mouse.clip(),
                        WM_INPUT if mouse.captured => {
                            if let Some((x, y)) = raw_motion(message.lParam) {
                                raw.0 = raw.0.saturating_add(x);
                                raw.1 = raw.1.saturating_add(y);
                            }
                        }
                        WM_LBUTTONDOWN if !mouse.captured && game.death.is_none() => {
                            mouse.capture()
                        }
                        WM_MOUSEWHEEL if mouse.captured => {
                            cycle = if (message.wParam >> 16) as i16 > 0 {
                                -1
                            } else {
                                1
                            }
                        }
                        _ => {}
                    }
                    if can_fire && let Some(key) = mouse_button(&message, false) {
                        primary |= key == settings.bindings[6];
                        alternate |= key == settings.bindings[7];
                    }
                }
                if message.hwnd == window
                    && message.message == WM_KEYDOWN
                    && message.lParam & (1 << 30) == 0
                {
                    let key = message.wParam as u16;
                    primary |= key == settings.bindings[6];
                    alternate |= key == settings.bindings[7];
                    match key {
                        VK_ESCAPE if let Some(frontend) = frontend.as_deref_mut() => {
                            mouse.release();
                            if !run_frontend(
                                window,
                                map,
                                frontend,
                                &hud,
                                &mut presenter,
                                &mut audio,
                                settings,
                                settings_path,
                                &mut windowed,
                                obs,
                                true,
                            )? {
                                return Ok(());
                            }
                            frame = Frame::hardware(map);
                            tick = Instant::now();
                            stats = FrameStats::new(tick);
                            primary = false;
                            alternate = false;
                            jump = false;
                            dodge = None;
                            raw = (0, 0);
                            if GetForegroundWindow() == window {
                                mouse.capture();
                            }
                        }
                        VK_ESCAPE if mouse.captured => mouse.release(),
                        VK_ESCAPE => {
                            DestroyWindow(window);
                            return Ok(());
                        }
                        VK_TAB if game.death.is_none() => {
                            start = (start + 1) % map.starts.len();
                            occupied_portal = None;
                            camera = Camera::from_start(map.starts[start]);
                            walker = Walker::spawn(&map.collision, map.starts[start].position);
                        }
                        key if key == u16::from(b'R') && !settings.bindings.contains(&key) => {
                            game.reset_player(&map.arsenal);
                            occupied_portal = None;
                            camera = Camera::from_start(map.starts[start]);
                            walker = Walker::spawn(&map.collision, map.starts[start].position);
                        }
                        key if key == settings.bindings[4] => jump = true,
                        key if settings.bindings[..4].contains(&key) => {
                            let pressed = Instant::now();
                            if last_direction.is_some_and(|(previous, time)| {
                                key == previous
                                    && pressed.duration_since(time) <= Duration::from_millis(250)
                            }) {
                                dodge = Some(key);
                                last_direction = None;
                            } else {
                                last_direction = Some((key, pressed));
                            }
                        }
                        VK_LEFT => camera.yaw -= 0.06,
                        VK_RIGHT => camera.yaw += 0.06,
                        VK_UP => camera.pitch += 0.05,
                        VK_DOWN => camera.pitch -= 0.05,
                        key if key == u16::from(b'F') && !settings.bindings.contains(&key) => {
                            primary = true
                        }
                        key if key == u16::from(b'N') && !settings.bindings.contains(&key) => {
                            music_muted = !music_muted
                        }
                        key if (u16::from(b'0')..=u16::from(b'9')).contains(&key) => {
                            slot = Some((key - u16::from(b'0')) as u8)
                        }
                        key if key == u16::from(b'M') && !settings.bindings.contains(&key) => {
                            muted = !muted
                        }
                        VK_F3 => {
                            settings.show_fps = !settings.show_fps;
                            save_settings(settings, settings_path);
                        }
                        VK_F8 if game.death.is_none() && game.ordnance.guided_view().is_none() => {
                            game.place_training_target(map, &camera);
                        }
                        VK_F11 => {
                            toggle_borderless(window, &mut windowed);
                            settings.fullscreen = windowed.is_some();
                            save_settings(settings, settings_path);
                        }
                        VK_F2 if game.death.is_none() => {
                            fly = !fly;
                            if !fly {
                                occupied_portal = None;
                                camera = Camera::from_start(map.starts[start]);
                                walker = Walker::spawn(&map.collision, map.starts[start].position);
                            }
                        }
                        _ => {}
                    }
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            let now = Instant::now();
            let dt = (now - tick).as_secs_f32().min(0.06);
            tick = now;
            game.base_fov = settings.field_of_view as f32;
            game.hud_scale = settings.hud_scale as f32 / 100.0;
            let [forward, back, left, right, jump_key, walk, fire, alt_fire] = settings.bindings;
            let mut movement_audio = MotionEvents::default();
            let focused = GetForegroundWindow() == window;
            let guiding = game.ordnance.guided_view().is_some();
            if guiding && pilot_view.is_none() {
                pilot_view = Some((camera.yaw, camera.pitch));
            }
            if let Some(death) = &mut game.death {
                if focused && primary && game.time - death.started >= 1.0 {
                    game.reset_player(&map.arsenal);
                    camera = Camera::from_start(map.starts[start]);
                    walker = Walker::spawn(&map.collision, map.starts[start].position);
                    occupied_portal = None;
                } else if walker.position.2 >= kill_z && !walker.grounded {
                    walker.advance(&map.collision, Vec3::default(), None, dt);
                    death.position = walker.position;
                }
                primary = false;
            } else if focused {
                if mouse.captured {
                    let (x, y) = mouse.motion(raw);
                    let sensitivity =
                        0.0027 * (settings.sensitivity as f32 / 100.0) * game.field_of_view()
                            / 90.0;
                    camera.yaw += x as f32 * sensitivity;
                    camera.pitch +=
                        y as f32 * sensitivity * if settings.invert_mouse { 1.0 } else { -1.0 };
                }
                update_view(&mut camera, dt);
                let movement =
                    camera.forward() * axis(forward, back) + camera.right() * axis(right, left);
                if guiding {
                    if !fly {
                        walker.advance(&map.collision, Vec3::default(), None, dt);
                    }
                } else if fly {
                    let movement = movement + Vec3(0.0, 0.0, axis(jump_key, VK_CONTROL));
                    let speed = if key(walk) { 950.0 } else { 340.0 };
                    camera.position = camera.position
                        + movement * (speed * dt / movement.dot(movement).sqrt().max(1.0));
                } else {
                    let speed =
                        map.collision.profile.ground_speed * if key(walk) { 0.3 } else { 1.0 };
                    for mover in &mut map.movers {
                        mover.tick(&mut map.collision, &mut walker, dt);
                    }
                    let dodged = dodge.take().is_some_and(|direction_key| {
                        if key(walk) {
                            return false;
                        }
                        let direction = match direction_key {
                            key if key == forward => camera.forward(),
                            key if key == back => camera.forward() * -1.0,
                            key if key == left => camera.right() * -1.0,
                            _ => camera.right(),
                        };
                        walker.dodge(map.collision.profile, direction)
                    });
                    let swim = camera.look_direction() * axis(forward, back)
                        + camera.right() * (0.5 * axis(right, left))
                        + Vec3(0.0, 0.0, f32::from(key(jump_key) || key(VK_CONTROL)) * 0.5);
                    movement_audio = walker.advance_with_swim(
                        &map.collision,
                        movement * (speed / movement.dot(movement).sqrt().max(1.0)),
                        swim,
                        jump.then(|| game.jump_speed(map.collision.profile.jump_speed)),
                        dt,
                    );
                    movement_audio.jump |= dodged;
                    if let Some((position, yaw)) = crate::teleport::touched(
                        &map.teleporters,
                        walker.position,
                        &mut occupied_portal,
                    ) {
                        walker = Walker::spawn(&map.collision, position);
                        if let Some(yaw) = yaw {
                            camera.yaw = yaw;
                        }
                    }
                    if walker.position.2 < kill_z {
                        game.health = 0;
                        occupied_portal = None;
                        walker = Walker::spawn(&map.collision, map.starts[start].position);
                    }
                }
            } else {
                mouse.release();
            }

            jump = false;
            dodge = None;
            if !fly {
                camera.position = walker.eye(map.collision.profile, camera.right());
            }
            if let Some(speed) = movement_audio.landing {
                game.fall_damage(speed, game.jump_speed(map.collision.profile.jump_speed));
            }
            let shots = game.update(
                map,
                &camera,
                dt,
                Input {
                    primary: focused && (primary || ((mouse.captured || fire > 6) && key(fire))),
                    alternate: focused
                        && (alternate || ((mouse.captured || alt_fire > 6) && key(alt_fire))),
                    slot: slot.take(),
                    cycle: std::mem::take(&mut cycle),
                },
                focused && !fly,
                (!fly && game.health > 0).then_some(&mut walker),
            );
            primary = false;
            alternate = false;
            if game.ordnance.guided_view().is_none()
                && let Some((yaw, pitch)) = pilot_view.take()
            {
                camera.yaw = yaw;
                camera.pitch = pitch;
            }
            let reaction = if game.health == 0 && game.death.is_none() {
                game.begin_death(walker.position, camera.yaw, map)
            } else {
                game.pain_sound(map)
            };
            if let Some(audio) = &mut audio {
                audio.set_volume(settings.effects_volume, settings.music_volume);
                if let Some(sound) = reaction {
                    audio.play(sound);
                }
                if movement_audio.jump
                    && let Some(sound) = &map.player_audio.jump
                {
                    audio.play(std::sync::Arc::clone(sound));
                }
                let foot_zone = map
                    .collision
                    .regions
                    .at(walker.position - Vec3(0.0, 0.0, 38.0));
                if let Some(step) = movement_audio.footstep {
                    let sound = if foot_zone.water {
                        &map.player_audio.water_step
                    } else {
                        &map.player_audio.steps[step]
                    };
                    if let Some(sound) = sound {
                        audio.play(std::sync::Arc::clone(sound));
                    }
                }
                if !foot_zone.water
                    && movement_audio.landing.is_some_and(|speed| {
                        0.1 * (speed / map.collision.profile.jump_speed.max(1.0)).powi(2) > 0.01
                    })
                    && let Some(sound) = &map.player_audio.land
                {
                    audio.play(std::sync::Arc::clone(sound));
                }
                for sound in shots {
                    audio.play(sound);
                }
                use crate::game::Weapon;
                let weapon_loop = if game.ordnance.guided_view().is_some() {
                    map.arsenal
                        .get(&Weapon::Redeemer)
                        .and_then(|weapon| weapon.charge_loop.as_ref())
                } else if game.hammer_charging() {
                    map.arsenal
                        .get(&Weapon::Hammer)
                        .and_then(|weapon| weapon.charge_loop.as_ref())
                } else if game.pulse_beam.is_some() {
                    map.arsenal
                        .get(&Weapon::Pulse)
                        .and_then(|weapon| weapon.alternate.as_ref())
                } else if game.pulse_primary_since.is_some() {
                    map.arsenal
                        .get(&Weapon::Pulse)
                        .and_then(|weapon| weapon.fire.as_ref())
                } else if game.minigun_firing {
                    map.arsenal
                        .get(&Weapon::Minigun)
                        .and_then(|weapon| weapon.fire.as_ref())
                } else {
                    None
                };
                audio.set_weapon_loop(weapon_loop);
                audio.update(
                    &map.ambient,
                    camera.position,
                    camera.right(),
                    muted || !focused,
                    music_muted,
                );
            }
            let mut client = RECT::default();
            if IsIconic(window) == 0
                && GetClientRect(window, &mut client) != 0
                && client.right > 0
                && client.bottom > 0
            {
                let size = (client.right as u32, client.bottom as u32);
                let presenter = match &mut presenter {
                    Some(presenter) => presenter,
                    slot @ None => {
                        slot.insert(Presenter::new(window, size, obs).map_err(|error| {
                            io::Error::other(format!("Creating the VSync swap chain: {error}"))
                        })?)
                    }
                };
                frame.width = size.0 as usize;
                frame.height = size.1 as usize;
                frame.draw_practice(map, &camera, (now - started).as_secs_f32(), &game);
                let track = map
                    .music
                    .as_ref()
                    .filter(|_| {
                        !music_muted
                            && !muted
                            && audio.as_ref().is_some_and(|audio| audio.music_playing())
                    })
                    .map(|track| track.name.as_str());
                let label = settings
                    .show_fps
                    .then(|| stats.label(window, size, presenter.software));
                hud.draw(
                    &mut frame,
                    &game,
                    mouse.captured,
                    map.hud.ready(),
                    track,
                    label.as_deref(),
                );
                let presented = presenter
                    .draw(
                        size,
                        frame.commands.as_mut().expect("GPU frame"),
                        settings.vsync,
                    )
                    .map_err(|error| io::Error::other(format!("Presenting the frame: {error}")))?;
                if obs && !obs_announced {
                    println!("OBS capture mode: {}x{}", size.0, size.1);
                    obs_announced = true;
                }
                if presented {
                    stats.presented(Instant::now());
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            } else {
                // Keep pumping audio and messages while there is no visible frame to present.
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_frontend(
    window: HWND,
    game_map: &Map,
    frontend: &mut Frontend,
    hud: &Hud,
    presenter: &mut Option<Presenter>,
    audio: &mut Option<Audio>,
    settings: &mut Settings,
    settings_path: &Path,
    windowed: &mut Option<WINDOWPLACEMENT>,
    obs: bool,
    resume: bool,
) -> io::Result<bool> {
    let mut intro = (!resume && settings.intro)
        .then(|| frontend.intro.take())
        .flatten();
    let mut menu = Menu::default();
    menu.resume = resume;
    let logo = frontend.logo.as_ref().map(|(map, _)| map);
    let mut frame = Frame::hardware(
        intro
            .as_ref()
            .map(|intro| &intro.map)
            .or(logo)
            .unwrap_or(game_map),
    );
    let mut tick = Instant::now();
    let mut time = 0.0;
    if let Some(audio) = audio {
        audio.set_weapon_loop(None);
        if !resume {
            audio.set_track(
                intro
                    .as_ref()
                    .map(|intro| &intro.map)
                    .or(logo)
                    .and_then(|map| map.music.as_ref()),
            );
        }
    }
    unsafe {
        loop {
            let now = Instant::now();
            let dt = (now - tick).as_secs_f32().min(0.06);
            tick = now;
            time += dt;
            let mut client = RECT::default();
            GetClientRect(window, &mut client);
            let size = [client.right, client.bottom];
            let mut message = MSG::default();
            let mut skip = false;
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    return Ok(false);
                }
                if message.hwnd == window {
                    let key = (message.message == WM_KEYDOWN && message.lParam & (1 << 30) == 0)
                        .then_some(message.wParam as u16);
                    let button = mouse_button(&message, true);
                    let previous_display = (settings.fullscreen, settings.resolution);
                    let action = if intro.is_some() {
                        skip |= matches!(key, Some(VK_ESCAPE | VK_SPACE | VK_RETURN))
                            || button == Some(VK_LBUTTON);
                        Action::None
                    } else if let Some(key) = key {
                        if key == VK_F11 {
                            settings.fullscreen = windowed.is_none();
                            Action::Changed
                        } else {
                            menu.key(key, settings)
                        }
                    } else if let Some(button) = button {
                        let x = message.lParam as i16 as i32;
                        let y = (message.lParam >> 16) as i16 as i32;
                        menu.click(size, x, y, button, settings)
                    } else {
                        Action::None
                    };
                    match action {
                        Action::Play => return Ok(true),
                        Action::Quit => {
                            DestroyWindow(window);
                            return Ok(false);
                        }
                        Action::Changed => {
                            if previous_display.0 != settings.fullscreen
                                && windowed.is_some() != settings.fullscreen
                            {
                                toggle_borderless(window, windowed);
                            }
                            if !settings.fullscreen
                                && previous_display != (settings.fullscreen, settings.resolution)
                            {
                                set_window_size(window, settings.resolution);
                            }
                            if let Err(error) = settings.save(settings_path) {
                                eprintln!("{}: {error}", settings_path.display());
                                menu.status = "COULD NOT SAVE SETTINGS".into();
                            }
                            if let Some(audio) = audio {
                                frontend.play_menu_sound(audio);
                            }
                        }
                        Action::None => {}
                    }
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            let intro_frame = intro
                .as_mut()
                .filter(|_| !skip)
                .and_then(|intro| intro.update(dt));
            if intro.is_some() && intro_frame.is_none() {
                intro = None;
                time = 0.0;
                frame = Frame::hardware(logo.unwrap_or(game_map));
                if let Some(audio) = audio {
                    audio.set_track(logo.and_then(|map| map.music.as_ref()));
                }
            }
            let active = GetForegroundWindow() == window;
            if let Some(audio) = audio {
                audio.set_volume(settings.effects_volume, settings.music_volume);
                if let (Some(intro), Some((shot, voices))) = (&intro, &intro_frame) {
                    for voice in voices {
                        audio.play(voice.clone());
                    }
                    audio.update(
                        &intro.map.ambient,
                        shot.camera.position,
                        shot.camera.right(),
                        !active,
                        false,
                    );
                } else {
                    audio.update(&[], Vec3::default(), Vec3(0.0, 1.0, 0.0), !active, false);
                }
            }
            if IsIconic(window) != 0 || size[0] <= 0 || size[1] <= 0 {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            frame.width = size[0] as usize;
            frame.height = size[1] as usize;
            if let (Some(intro), Some((shot, _))) = (&intro, &intro_frame) {
                frame.draw_cinematic(
                    &intro.map,
                    &shot.camera,
                    intro.time,
                    shot.fov,
                    shot.roll,
                    shot.flash,
                );
                if intro.time < 6.0 {
                    hud.text(
                        &mut frame,
                        12,
                        size[1] - 30,
                        0.85,
                        "ESC TO SKIP",
                        [0.8, 0.8, 0.8],
                    );
                }
            } else {
                if !frontend.has_desktop()
                    && let Some((logo, camera)) = &frontend.logo
                {
                    frame.draw_cinematic(logo, camera, time, 90.0, 0.0, 1.0);
                } else {
                    frame.begin_ui(true);
                }
                menu.draw(frontend, &mut frame, hud, settings);
            }
            let size = (size[0] as u32, size[1] as u32);
            let presenter = match &mut *presenter {
                Some(presenter) => presenter,
                slot @ None => {
                    slot.insert(Presenter::new(window, size, obs).map_err(io::Error::other)?)
                }
            };
            if !presenter
                .draw(
                    size,
                    frame.commands.as_mut().expect("GPU frame"),
                    settings.vsync,
                )
                .map_err(io::Error::other)?
            {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn save_settings(settings: &Settings, path: &Path) {
    if let Err(error) = settings.save(path) {
        eprintln!("{}: {error}", path.display());
    }
}

fn mouse_button(message: &MSG, release: bool) -> Option<u16> {
    match (message.message, release) {
        (WM_LBUTTONDOWN, false) | (WM_LBUTTONUP, true) => Some(VK_LBUTTON),
        (WM_RBUTTONDOWN, false) | (WM_RBUTTONUP, true) => Some(VK_RBUTTON),
        (WM_MBUTTONDOWN, false) | (WM_MBUTTONUP, true) => Some(VK_MBUTTON),
        (WM_XBUTTONDOWN, false) | (WM_XBUTTONUP, true) => Some(if message.wParam >> 16 == 1 {
            VK_XBUTTON1
        } else {
            VK_XBUTTON2
        }),
        _ => None,
    }
}

fn set_window_size(window: HWND, resolution: usize) {
    let (width, height) = RESOLUTIONS[resolution];
    unsafe {
        let mut frame = RECT {
            right: width as i32,
            bottom: height as i32,
            ..Default::default()
        };
        AdjustWindowRectEx(&mut frame, WS_OVERLAPPEDWINDOW, 0, 0);
        let mut monitor = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(
            MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        ) != 0
        {
            let area = monitor.rcWork;
            let w = (frame.right - frame.left).min(area.right - area.left);
            let h = (frame.bottom - frame.top).min(area.bottom - area.top);
            SetWindowPos(
                window,
                null_mut(),
                (area.right + area.left - w) / 2,
                (area.bottom + area.top - h) / 2,
                w,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

struct FrameStats {
    since: Instant,
    frames: u32,
    fps: f32,
    refresh: Option<u32>,
    refresh_checked: Option<Instant>,
}

impl FrameStats {
    fn new(now: Instant) -> Self {
        Self {
            since: now,
            frames: 0,
            fps: 0.0,
            refresh: None,
            refresh_checked: None,
        }
    }

    fn presented(&mut self, now: Instant) {
        self.frames += 1;
        let elapsed = now.duration_since(self.since);
        if elapsed >= Duration::from_millis(500) {
            self.fps = self.frames as f32 / elapsed.as_secs_f32();
            self.frames = 0;
            self.since = now;
        }
    }

    fn label(&mut self, window: HWND, size: (u32, u32), software: bool) -> String {
        let now = Instant::now();
        if self
            .refresh_checked
            .is_none_or(|checked| now.duration_since(checked) >= Duration::from_secs(1))
        {
            self.refresh = monitor_refresh(window);
            self.refresh_checked = Some(now);
        }
        let refresh = self
            .refresh
            .map_or_else(|| "? Hz".to_owned(), |hz| format!("{hz} Hz"));
        let device = if software { "WARP" } else { "D3D11" };
        format!(
            "{:.0} FPS / {refresh}  {}x{}  {device}",
            self.fps, size.0, size.1
        )
    }
}

fn monitor_refresh(window: HWND) -> Option<u32> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let mut mode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    unsafe {
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor, &mut info.monitorInfo) == 0
            || EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) == 0
            || mode.dmDisplayFrequency <= 1
        {
            return None;
        }
    }
    Some(mode.dmDisplayFrequency)
}

fn toggle_borderless(window: HWND, windowed: &mut Option<WINDOWPLACEMENT>) {
    unsafe {
        if let Some(placement) = windowed.take() {
            SetWindowLongW(window, GWL_STYLE, (WS_OVERLAPPEDWINDOW | WS_VISIBLE) as i32);
            SetWindowPlacement(window, &placement);
            SetWindowPos(
                window,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        } else {
            let mut placement = WINDOWPLACEMENT {
                length: size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            };
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
            if GetWindowPlacement(window, &mut placement) == 0
                || GetMonitorInfoW(monitor, &mut info) == 0
            {
                return;
            }
            *windowed = Some(placement);
            let visibility = GetWindowLongW(window, GWL_STYLE) as u32 & WS_VISIBLE;
            SetWindowLongW(window, GWL_STYLE, (WS_POPUP | visibility) as i32);
            let area = info.rcMonitor;
            SetWindowPos(
                window,
                HWND_TOP,
                area.left,
                area.top,
                area.right - area.left,
                area.bottom - area.top,
                SWP_FRAMECHANGED,
            );
        }
    }
}

struct Mouse {
    window: HWND,
    captured: bool,
    center: POINT,
}

impl Mouse {
    fn new(window: HWND) -> Self {
        let mouse = RAWINPUTDEVICE {
            usUsagePage: 1,
            usUsage: 2,
            dwFlags: 0,
            hwndTarget: window,
        };
        unsafe {
            RegisterRawInputDevices(&mouse, 1, size_of::<RAWINPUTDEVICE>() as u32);
        }
        Self {
            window,
            captured: false,
            center: POINT::default(),
        }
    }

    fn capture(&mut self) {
        if self.captured {
            return;
        }
        self.captured = true;
        self.clip();
        unsafe {
            ShowCursor(0);
        }
    }

    fn clip(&mut self) {
        unsafe {
            let mut rect = RECT::default();
            if GetClientRect(self.window, &mut rect) == 0 || rect.right == 0 || rect.bottom == 0 {
                return;
            }
            let mut origin = POINT::default();
            if ClientToScreen(self.window, &mut origin) == 0 {
                return;
            }
            rect.left += origin.x;
            rect.right += origin.x;
            rect.top += origin.y;
            rect.bottom += origin.y;
            self.center = POINT {
                x: (rect.left + rect.right) / 2,
                y: (rect.top + rect.bottom) / 2,
            };
            ClipCursor(&rect);
            SetCursorPos(self.center.x, self.center.y);
        }
    }

    fn motion(&mut self, raw: (i32, i32)) -> (i32, i32) {
        let mut cursor = POINT::default();
        unsafe {
            if GetCursorPos(&mut cursor) == 0 {
                return raw;
            }
            let legacy = (cursor.x - self.center.x, cursor.y - self.center.y);
            if legacy != (0, 0) {
                SetCursorPos(self.center.x, self.center.y);
            }
            if raw == (0, 0) { legacy } else { raw }
        }
    }

    fn release(&mut self) {
        if !self.captured {
            return;
        }
        self.captured = false;
        unsafe {
            ClipCursor(std::ptr::null());
            ShowCursor(1);
        }
    }
}

impl Drop for Mouse {
    fn drop(&mut self) {
        self.release();
    }
}

fn raw_motion(data: LPARAM) -> Option<(i32, i32)> {
    unsafe {
        let mut input = RAWINPUT::default();
        let mut size = size_of::<RAWINPUT>() as u32;
        let read = GetRawInputData(
            data as _,
            RID_INPUT,
            (&raw mut input).cast(),
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        );
        if read == u32::MAX || input.header.dwType != RIM_TYPEMOUSE {
            return None;
        }
        let mouse = input.data.mouse;
        (mouse.usFlags & MOUSE_MOVE_ABSOLUTE == 0).then_some((mouse.lLastX, mouse.lLastY))
    }
}

fn key(code: u16) -> bool {
    // GetAsyncKeyState uses the high bit for the current key state.
    unsafe { GetAsyncKeyState(i32::from(code)) < 0 }
}

fn axis(positive: u16, negative: u16) -> f32 {
    (i32::from(key(positive)) - i32::from(key(negative))) as f32
}

fn update_view(camera: &mut Camera, dt: f32) {
    camera.yaw =
        (camera.yaw + axis(VK_RIGHT, VK_LEFT) * dt * 2.2).rem_euclid(std::f32::consts::TAU);
    camera.pitch = (camera.pitch + axis(VK_UP, VK_DOWN) * dt * 1.8).clamp(-1.5, 1.5);
}
