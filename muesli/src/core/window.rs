use sdl2::VideoSubsystem;
use sdl2::video::Window;
use super::graphics::graphics_context::GraphicsContext;
use crate::core::graphics::renderer::Renderer;

pub struct AppWindow
{
    sdl_window: Window,
    graphics_context: GraphicsContext
}

impl AppWindow
{
    pub fn new(video: &VideoSubsystem, 
        args: AppWindowArgs) -> Self
    {
        let sdl_window = video
            .window(&args.name[..], args.width, args.height)
            .opengl()
            .resizable()
            .build()
            .unwrap();
        let graphics_context = GraphicsContext::new(&sdl_window, 
            video, 
            args.width as i32, 
            args.height as i32);
        AppWindow{sdl_window, graphics_context}
    }
    
    pub fn update_view(&self, renderer: Box<&mut dyn Renderer>)
    {
        renderer.render(&self.graphics_context);
        self.sdl_window.gl_swap_window();
    }
}

pub struct AppWindowArgs
{
    name: String,
    width: u32,
    height: u32
}

impl AppWindowArgs
{
    pub fn new(name: String, width: u32, height:u32) -> Self
    {
        AppWindowArgs{name, width, height}
    }
}