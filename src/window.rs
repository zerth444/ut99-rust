use std::io;
use std::ptr::null_mut;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Input::{
    GetRawInputData, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::collision::{MotionEvents, Walker};
use crate::game::{Input, Practice};
use crate::hud::Hud;
use crate::map::{Map, Vec3};
use crate::render::{Camera, Frame, HEIGHT, WIDTH};

pub fn show(map: &mut Map, start: usize, title: &str) -> io::Result<()> {
    let class: Vec<u16> = "Ut99RustViewer\0".encode_utf16().collect();
    let title: Vec<u16> = title.encode_utf16().chain([0]).collect();

    // The HWND, its device contexts and the message pump all live on this thread.
    unsafe {
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
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
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
        message_loop(window, map, start)
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
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

unsafe fn message_loop(window: HWND, map: &mut Map, mut start: usize) -> io::Result<()> {
    let mut frame = Frame::new();
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
    let mut audio = crate::sound::device::Audio::open(map.music.as_ref());
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
    let started = Instant::now();
    let mut tick = started;

    let bitmap = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: WIDTH as i32,
            biHeight: -(HEIGHT as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };

    unsafe {
        loop {
            let mut raw = (0i32, 0i32);
            let mut message = MSG::default();
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    return Ok(());
                }
                if message.hwnd == window {
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
                        WM_LBUTTONDOWN => {
                            if mouse.captured {
                                primary = true;
                            } else {
                                mouse.capture();
                            }
                        }
                        WM_RBUTTONDOWN if mouse.captured => alternate = true,
                        WM_MOUSEWHEEL if mouse.captured => {
                            cycle = if (message.wParam >> 16) as i16 > 0 {
                                -1
                            } else {
                                1
                            }
                        }
                        _ => {}
                    }
                }
                if message.hwnd == window
                    && message.message == WM_KEYDOWN
                    && message.lParam & (1 << 30) == 0
                {
                    match message.wParam as u16 {
                        VK_ESCAPE if mouse.captured => mouse.release(),
                        VK_ESCAPE => {
                            DestroyWindow(window);
                            return Ok(());
                        }
                        VK_TAB => {
                            start = (start + 1) % map.starts.len();
                            occupied_portal = None;
                            camera = Camera::from_start(map.starts[start]);
                            walker = Walker::spawn(&map.collision, map.starts[start].position);
                        }
                        key if key == u16::from(b'R') => {
                            game.reset_player(&map.arsenal);
                            occupied_portal = None;
                            camera = Camera::from_start(map.starts[start]);
                            walker = Walker::spawn(&map.collision, map.starts[start].position);
                        }
                        VK_SPACE => jump = true,
                        key if [
                            u16::from(b'W'),
                            u16::from(b'S'),
                            u16::from(b'A'),
                            u16::from(b'D'),
                        ]
                        .contains(&key) =>
                        {
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
                        key if key == u16::from(b'F') => primary = true,
                        key if key == u16::from(b'N') => music_muted = !music_muted,
                        key if (u16::from(b'0')..=u16::from(b'9')).contains(&key) => {
                            slot = Some((key - u16::from(b'0')) as u8)
                        }
                        key if key == u16::from(b'M') => muted = !muted,
                        VK_F2 => {
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
            let mut movement_audio = MotionEvents::default();
            if GetForegroundWindow() == window {
                if mouse.captured {
                    let (x, y) = mouse.motion(raw);
                    camera.yaw += x as f32 * 0.0027;
                    camera.pitch -= y as f32 * 0.0027;
                }
                update_view(&mut camera, dt);
                let movement = camera.forward() * axis(b'W' as u16, b'S' as u16)
                    + camera.right() * axis(b'D' as u16, b'A' as u16);
                if fly {
                    let movement = movement + Vec3(0.0, 0.0, axis(VK_SPACE, VK_CONTROL));
                    let speed = if key(VK_SHIFT) { 950.0 } else { 340.0 };
                    camera.position = camera.position
                        + movement * (speed * dt / movement.dot(movement).sqrt().max(1.0));
                } else {
                    let speed =
                        map.collision.profile.ground_speed * if key(VK_SHIFT) { 0.3 } else { 1.0 };
                    for mover in &mut map.movers {
                        mover.tick(&mut map.collision, &mut walker, dt);
                    }
                    let dodged = dodge.take().is_some_and(|direction_key| {
                        if key(VK_SHIFT) {
                            return false;
                        }
                        let direction = match direction_key as u8 {
                            b'W' => camera.forward(),
                            b'S' => camera.forward() * -1.0,
                            b'A' => camera.right() * -1.0,
                            _ => camera.right(),
                        };
                        walker.dodge(map.collision.profile, direction)
                    });
                    movement_audio = walker.advance(
                        &map.collision,
                        movement * (speed / movement.dot(movement).sqrt().max(1.0)),
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
                        game.reset_player(&map.arsenal);
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
            let focused = GetForegroundWindow() == window;
            let shots = game.update(
                map,
                &camera,
                dt,
                Input {
                    primary: focused && (primary || (mouse.captured && key(VK_LBUTTON))),
                    alternate: focused && (alternate || (mouse.captured && key(VK_RBUTTON))),
                    slot: slot.take(),
                    cycle: std::mem::take(&mut cycle),
                },
                focused && !fly,
            );
            primary = false;
            alternate = false;
            if game.health == 0 {
                game.reset_player(&map.arsenal);
                camera = Camera::from_start(map.starts[start]);
                walker = Walker::spawn(&map.collision, map.starts[start].position);
                occupied_portal = None;
            }
            if let Some(audio) = &mut audio {
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
                audio.update(
                    &map.ambient,
                    camera.position,
                    camera.right(),
                    muted || !focused,
                    music_muted,
                );
            }
            if IsIconic(window) == 0 {
                frame.draw_practice(map, &camera, (now - started).as_secs_f32(), &game);
                let mut client = RECT::default();
                if GetClientRect(window, &mut client) != 0 {
                    let dc = GetDC(window);
                    if dc.is_null() {
                        return Err(io::Error::last_os_error());
                    }
                    StretchDIBits(
                        dc,
                        0,
                        0,
                        client.right,
                        client.bottom,
                        0,
                        0,
                        WIDTH as i32,
                        HEIGHT as i32,
                        frame.pixels.as_ptr().cast(),
                        &bitmap,
                        DIB_RGB_COLORS,
                        SRCCOPY,
                    );
                    let track = map
                        .music
                        .as_ref()
                        .filter(|_| {
                            !music_muted
                                && !muted
                                && audio.as_ref().is_some_and(|audio| audio.music_playing())
                        })
                        .map(|track| track.name.as_str());
                    hud.draw(dc, client, &game, mouse.captured, map.hud.ready(), track);
                    ReleaseDC(window, dc);
                }
            }
            std::thread::sleep(Duration::from_millis(10).saturating_sub(tick.elapsed()));
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
