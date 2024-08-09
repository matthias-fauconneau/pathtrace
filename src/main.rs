#![feature(slice_from_ptr_range, let_chains)]
//#![allow(incomplete_features)]#![feature(slice_from_ptr_range, array_chunks, iter_array_chunks, inherent_associated_types, anonymous_lifetime_in_impl_trait, f16, let_chains)]
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
			&[exr::prelude::Sample::F32(v)] => [v, 1.],
			&[exr::prelude::Sample::F16(v)] => [f32::from(v), 1.],
			v => unimplemented!("{v:?}")
		};
	}}
Ok(image)
}


pub use vulkan::{default, Error, Result, throws};
use image::{rgba, rgba8, sRGB8_OETF12, oetf8_12};
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

pub fn n(size: uint2, p: uint2, d: int2) -> uint2 { xy{x:((p.x as i32+d.x) as u32+size.x)%size.x,y:(p.y as i32+d.y) as u32} }
pub fn draw_cross(target: &mut Image<impl std::ops::DerefMut<Target=[rgba8]>>, center: uint2, color: rgba8) {
	let mut set = |d| if let Some(p) = target.get_mut(n(target.size, center, d)) {*p = color; };
	for y in -64..64 { set(xy{x: -1, y}); set(xy{x: 0, y}); set(xy{x: 1, y}); }
	for x in -64..64 { set(xy{x, y: -1}); set(xy{x, y: 0}); set(xy{x, y: 1}); }
}

fn main() -> Result {
	let mut min_max = None;
	let mut images = vec![];
	let mut cross = vec![];
	for arg in std::env::args().skip(1) {
		if let Some(("",xy)) = arg.split_once("cross:") && let Some((x,y)) = xy.split_once(",") { cross.push(xy{x,y}.map(|x| x.parse().unwrap())); }
		else {
			let mut image = if let Ok(image) = exr(&arg) {
				let [Some(&[min,_]), Some(&[max,_])] = [image.data.iter().min_by(|[a,_],[b,_]| f32::total_cmp(a,b)), image.data.iter().max_by(|[a,_],[b,_]| f32::total_cmp(a,b))] else {unreachable!()};
				println!("{} {}", min, max);
				if false { // Prints histogram
					let mut histogram = vec![0; 0x100];
					for [z,_] in &image.data { histogram[((z-min)/(max-min)*(0xFF as f32)) as usize] += 1; }
					println!("{histogram:?}");
				}
				// Tonemaps all float images to match first
				let [min, max] = min_max.unwrap_or([min, max]);
				min_max = Some([min, max]);
				let oetf = &sRGB8_OETF12;
				Image::from_iter(image.size, image.data.iter().map(|&[z,a]| {
					let z = oetf8_12(oetf, ((z-min)/(max-min)).clamp(0., 1.));
					assert!(a >= 0. && a <= 1., "{a}");
					rgba{r: z, g: z, b: z, a: (a*(0xFF as f32)) as u8}
				}))
			} else {
				::image::rgba8(arg).map(|rgba{r,g,b,..}| rgba{r,g,b,a:0xFF})
			};
			for &cross in &cross { draw_cross(&mut image, cross, rgba{r: 0xFF, g: 0, b: 0xFF, a: 0xFF}); }
			images.push(image);
		}
	}
	app::run(std::env::args().skip(1).collect::<Vec<_>>().join(", "), Box::new(move |context,commands| Ok(Box::new(App::new(context, commands, &images)?))))
}
