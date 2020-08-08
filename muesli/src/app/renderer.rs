

use crate::core::graphics::renderer::Renderer;
use crate::core::graphics::graphics_context::GraphicsContext;

pub struct AppRenderer{
    color: (f32, f32, f32, f32),
    reverse: bool
}

impl AppRenderer
{
    pub fn new() -> Self
    {
        let color = (0.0, 0.0, 0.0, 0.0);
        AppRenderer{color, reverse: false}
    }

    fn adjust_color(&mut self)
    {
        let c; 
        
        if self.reverse
        {
            c = self.color.0 - 0.01;
        } else
        {
            c = self.color.0 + 0.01;
        }
       
        self.color = (c, c, c, 1.0);
        if c>1.0 || c<0.0
        {
            self.reverse = !self.reverse;
        }
    }
}

impl Renderer for AppRenderer{
    fn render(&mut self, _context: &GraphicsContext)
    {
        unsafe {
            gl::ClearColor(self.color.0, self.color.1, self.color.2, self.color.3);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }

        self.adjust_color();
    }
}