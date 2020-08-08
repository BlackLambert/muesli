use std::os::raw::c_void;
use sdl2::video::Window;
use sdl2::video::GLContext;
use sdl2::VideoSubsystem;


pub struct GraphicsContext
{
    _gl_context: GLContext,
}

impl GraphicsContext
{
    pub fn new(window: &Window, video: &VideoSubsystem, viewport_width:i32, viewport_height:i32) -> Self
    {
        let _gl_context = window.gl_create_context().unwrap();
        gl::load_with(|s| video.gl_get_proc_address(s) as *const c_void);
        let gl_attr = video.gl_attr();

        gl_attr.set_context_profile(sdl2::video::GLProfile::Core);
        gl_attr.set_context_version(4, 5);
        unsafe{
            gl::Viewport(0,0,viewport_width, viewport_height);
        }
        GraphicsContext{_gl_context}
    }
    
}