use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use std::sync::mpsc::Sender;

pub fn open_window_thread(sender: Sender<Keycode>) {
    std::thread::spawn(move || {
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let window = video.window("SDL2", 320, 240).resizable().build().unwrap();
        let mut canvas = window.into_canvas().present_vsync().build().unwrap();
        let mut event_pump = sdl.event_pump().unwrap();
        'main: loop {
            for event in event_pump.poll_iter() {
                match event {
                    Event::Quit { .. } => break 'main,
                    Event::KeyDown {
                        keycode: Some(keycode),
                        repeat: false,
                        ..
                    } => sender.send(keycode).unwrap(),
                    _ => {}
                }
            }
            canvas.clear();
            canvas.present();
        }
    });
}
