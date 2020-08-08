extern crate sdl2;
extern crate gl;
mod core;
mod app;

fn main() {
    app::App::new().start();
}
