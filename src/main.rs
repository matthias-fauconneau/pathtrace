#![allow(incomplete_features)]#![feature(slice_from_ptr_range, array_chunks, iter_array_chunks, inherent_associated_types, anonymous_lifetime_in_impl_trait, f16)]
mod app; mod vulkan; mod shader;
use image::{Image, xy};
fn exr(path: impl AsRef<std::path::Path>) -> Result<Image<Box<[[f32; 2]]>>> {
	let exr = exr::prelude::read_first_flat_layer_from_file(path)?.layer_data;
	let size = {let exr::prelude::Vec2(x,y) = exr.size; xy{x: x as u32,y: y as _}};
	let mut image = Image::<Box<[[f32; 2]]>>::uninitialized(size);
	for y in 0..size.y { for x in 0..size.x {
		image[xy{x,y}] = match exr.sample_vec_at(exr::prelude::Vec2(x as _,y as _)).as_slice() {
			&[exr::prelude::Sample::F32(z), exr::prelude::Sample::F32(a)] => [z,a],
			&[exr::prelude::Sample::F16(z), exr::prelude::Sample::F16(a)] => [f32::from(z), f32::from(a)],
			&[exr::prelude::Sample::F32(v)] => [v; 2],
			&[exr::prelude::Sample::F16(v)] => [f32::from(v); 2],
			v => unimplemented!("{v:?}")
		};
	}}
Ok(image)
}


pub use vulkan::{default, Error, Result, throws};
use image::{rgba, rgba8};
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
	let images = std::env::args().skip(1).map(|ref path|
		if let Ok(image) = exr(path) {
			let [Some(&[min,_]), Some(&[max,_])] = [image.data.iter().min_by(|[a,_],[b,_]| f32::total_cmp(a,b)), image.data.iter().max_by(|[a,_],[b,_]| f32::total_cmp(a,b))] else {unreachable!()};
			Image::from_iter(image.size, image.data.iter().map(|&[z,_a]| {
				let z = (((z-min)/(max-min))*(0xFF as f32)) as u8; // FIXME: OETF
				rgba{r: z, g: z, b: z, a: 0xFF}
			}))
		} else { ::image::rgba8(path) }
	).collect::<Box<_>>();
	app::run(std::env::args().skip(1).collect::<Vec<_>>().join(", "), Box::new(move |context,commands| Ok(Box::new(App::new(context, commands, &images)?))))
}
