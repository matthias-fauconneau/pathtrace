#![feature(slice_from_ptr_range)]
mod app; mod vulkan; mod shader;

pub use vulkan::{default, Error, Result, throws};
use image::{Image, rgba, rgba8, sRGB8_OETF12, oetf8_12};
use {std::sync::Arc, vulkan::Context, vulkano::{memory::allocator::{AllocationCreateInfo, MemoryTypeFilter}, command_buffer::{CopyBufferToImageInfo, RecordingCommandBuffer as Commands}, buffer::{Buffer, BufferCreateInfo, BufferUsage}, image::{Image as GPUImage, ImageType, ImageCreateInfo, ImageUsage, view::ImageView, sampler::{Sampler, SamplerCreateInfo, Filter}}, format::Format, descriptor_set::WriteDescriptorSet}};
use app::{uint2, int2, Event};
use winit::{event::{Event::WindowEvent,WindowEvent::KeyboardInput,KeyEvent,ElementState::Pressed},keyboard::{Key::Named,NamedKey::{ArrowLeft,ArrowRight}}};
crate::shader!{quad, Quad}

struct App {
	quad: Quad,
	images: Vec<Arc<GPUImage>>,
	index: usize,
}

impl App {
	#[throws] fn new<D: AsRef<[rgba8]>>(context@Context{memory_allocator,..}: &Context, commands: &mut Commands, images: &[Image<D>]) -> Self { Self{
		quad: Quad::new(context)?,
		images: images.iter().map(|Image{size, data, stride}| {
			let image = vulkano::image::Image::new(
				memory_allocator.clone(),
				ImageCreateInfo{
					image_type: ImageType::Dim2d,
					format: {assert_eq!(std::mem::size_of::<rgba8>(), 4); Format::R8G8B8A8_SRGB},
					extent: [size.x, size.y, 1],
					usage: ImageUsage::TRANSFER_DST|ImageUsage::SAMPLED,
					..default()
				},
				default()
			).unwrap();
			let buffer = Buffer::new_slice::<rgba8>(
				memory_allocator.clone(),
				BufferCreateInfo{usage: BufferUsage::TRANSFER_SRC, ..default()},
				AllocationCreateInfo{memory_type_filter: MemoryTypeFilter::PREFER_DEVICE|MemoryTypeFilter::HOST_SEQUENTIAL_WRITE, ..default()},
				(size.x * size.y) as u64
			).unwrap();
			{
				let mut write_guard = buffer.write().unwrap();
				assert_eq!(*stride, size.x);
				write_guard.copy_from_slice(&data.as_ref());
			}
			commands.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(buffer, image.clone())).unwrap();
			image
		}).collect(),
		index: 0,
	}}
}

impl app::App<()> for App {
	fn render(&mut self, context: &Context, commands: &mut Commands, _async: Option<()>, target: Arc<ImageView>) -> Result<bool> {
		let sampler = Sampler::new(context.device.clone(), SamplerCreateInfo{mag_filter: Filter::Linear, min_filter: Filter::Linear, ..default()}).unwrap();
		self.quad.begin_rendering(context, commands, target.clone(), &[
			WriteDescriptorSet::image_view(0, ImageView::new_default(self.images[self.index].clone())?),
			WriteDescriptorSet::sampler(1, sampler.clone())
		])?;
		unsafe{commands.draw(4, 1, 0, 0)}?;
		commands.end_rendering()?;
		Ok(false)
	}
	fn event(&mut self, _size: uint2, _: int2, _mouse_buttons: u32, event: Event<()>) -> bool {
		let WindowEvent{event, ..} = &event else {return false;};
		match event {
			KeyboardInput{event: KeyEvent{logical_key: Named(key), state: Pressed, ..}, ..} => match key {
				ArrowLeft => { self.index = (self.index+self.images.len()-1)%self.images.len(); true },
				ArrowRight => { self.index = (self.index+1)%self.images.len(); true },
				_ => false,
			}
			_ => false,
		}
	}
}

fn main() -> Result {
	let mut min_max = None;
	let mut images = vec![];
	for arg in std::env::args().skip(1) {
		let start = std::time::Instant::now();
		let image = image::f32(arg)?;
		println!("{}", start.elapsed().as_millis());
		let image = {
			let [Some(&min), Some(&max)] = [image.data.iter().filter(|&&v| v>=0.).min_by(|a,b| f32::total_cmp(a,b)), image.data.iter().max_by(|a,b| f32::total_cmp(a,b))] else {unreachable!()};
			println!("{} {}", min, max);
			// Tonemaps all float images to match first
			let [min, max] = min_max.unwrap_or([min, max]);
			min_max = Some([min, max]);
			let oetf = &sRGB8_OETF12;
			Image::from_iter(image.size, image.data.iter().map(|&v| {
				let v = oetf8_12(oetf, ((v-min)/(max-min)).clamp(0., 1.));
				rgba{r: v, g: v, b: v, a: 0xFF}
			}))
		};
		images.push(image);
	}
	app::run(std::env::args().skip(1).collect::<Vec<_>>().join(", "), Box::new(move |context,commands| Ok(Box::new(App::new(context, commands, &images)?))))
}
