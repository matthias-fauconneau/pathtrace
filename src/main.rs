#![feature(slice_from_ptr_range)]
#![feature(iter_next_chunk)]
use ui::{Result, size, int2, Widget, EventContext, Event::{self, Key, Idle}, vulkan, shader};
use vulkan::{Context, Commands, Arc, ImageView, Image as GPUImage, image, WriteDescriptorSet, linear};
use image::{load_rgb8, rgba8};
shader!{view}

struct App {
	pass: view::Pass,
	images: Box<[std::path::PathBuf]>,
	image: Arc<GPUImage>,
	index: usize,
}
impl App {
	fn new(context: &Context, commands: &mut Commands) -> Result<Self> { 
		let [_,path] = std::env::args().next_chunk().unwrap();
		let images = std::fs::read_dir(path)?.map(|e| e.unwrap().path()).collect::<Box<_>>();
		let image = image(context, commands, load_rgb8(&images[0]).map(|v| rgba8::from(v)).as_ref())?;
		Ok(Self{pass: view::Pass::new(context, false)?, images, image, index: 0})
	}
}

impl Widget for App { 
fn paint(&mut self, context: &Context, commands: &mut Commands, target: Arc<ImageView>, _: size, _: int2) -> Result<()> {
	let Self{pass, image, ..} = self;
	pass.begin_rendering(context, commands, target.clone(), None, true, &view::Uniforms::empty(), &[
		WriteDescriptorSet::image_view(0, ImageView::new_default(image.clone())?),
		WriteDescriptorSet::sampler(1, linear(context)),
	])?;
	unsafe{commands.draw(3, 1, 0, 0)}?;
	commands.end_rendering()?;
	Ok(())
}
fn event(&mut self, context: &Context, commands: &mut Commands, _size: size, _: &mut EventContext, event: &Event) -> Result<bool> {
	let need_paint = match event {
		Key('←') => { self.index = (self.index+self.images.len()-1)%self.images.len(); true },
		Key('→')|Idle => { self.index = (self.index+1)%self.images.len(); true },
		_ => false,
	};
	if need_paint { self.image = image(context, commands, load_rgb8(&self.images[self.index]).map(|v| rgba8::from(v)).as_ref())?; }
	Ok(need_paint)
}
}

fn main() -> Result { ui::run("view", Box::new(|context, commands| Ok(Box::new(App::new(context, commands)?)))) }
