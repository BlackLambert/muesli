const START_WINDOW_WIDTH: u32 = 800;
const START_WINDOW_HIGHT: u32 = 600;
const APP_NAME: &str = "Muesli";

pub mod renderer;

use sdl2::Sdl;
use crate::core::window::{AppWindow, AppWindowArgs};
use self::renderer::AppRenderer;


pub struct App
{
    sdl: Sdl,
    renderer: AppRenderer,
    window: AppWindow
}

impl App{
    pub fn new() -> Self
    {
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let window_args = AppWindowArgs::new(String::from(APP_NAME), START_WINDOW_WIDTH, START_WINDOW_HIGHT);
        let window = AppWindow::new(&video, window_args);

        let renderer = AppRenderer::new();
        App{sdl, renderer, window}
    }

    pub fn start(&mut self)
    {
        let mut event_pump = self.sdl.event_pump().unwrap();
        'main: loop {
            for event in event_pump.poll_iter() {
                match event {
                    sdl2::event::Event::Quit {..} => break 'main,
                    _ => {},
                }
            }

            self.window.update_view(Box::new(&mut self.renderer));
        }
    }
}
