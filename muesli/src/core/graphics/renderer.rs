
use super::graphics_context::GraphicsContext;

pub trait Renderer{
    fn render(&mut self, context: &GraphicsContext);
}

