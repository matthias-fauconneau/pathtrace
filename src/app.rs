use crate::{default, Error, Result};
use vector::{xy, uint2, int2};
use winit::{event_loop::EventLoop, window::{WindowBuilder, Fullscreen},
	event::{Event, WindowEvent::{Resized,RedrawRequested,CloseRequested,KeyboardInput,CursorMoved,CursorEntered}, KeyEvent, ElementState::Pressed},
	keyboard::{Key::{Named, Character}, NamedKey::Escape}};
use {std::sync::Arc, vulkano::{VulkanLibrary, Validated, VulkanError, instance::{Instance, InstanceCreateInfo, InstanceExtensions},
	device::{Device, DeviceCreateInfo, DeviceFeatures, DeviceExtensions, physical::PhysicalDeviceType, QueueCreateInfo, QueueFlags},
	memory::allocator::StandardMemoryAllocator,
	command_buffer::{allocator::StandardCommandBufferAllocator, CommandBufferLevel, CommandBufferBeginInfo, CommandBufferUsage},
	descriptor_set::allocator::StandardDescriptorSetAllocator,
	swapchain::{Swapchain, SwapchainCreateInfo, Surface, SwapchainPresentInfo, acquire_next_image},
	command_buffer::RecordingCommandBuffer,
	image::{ImageUsage, view::ImageView}, format::Format,
	sync::{future::{GpuFuture, FenceSignalFuture}, now},
}};
use crate::vulkan::Context;


pub trait App<T> {

	fn event(&mut self, _size: uint2, _pointer_position: int2, _mouse_buttons: u32, _event: Event<()>) -> bool { false }

	fn render(&mut self, context: &Context, commands: &mut RecordingCommandBuffer, join: Option<T>, target: Arc<ImageView>, depth: Option<Arc<ImageView>>) -> Result<bool>;

}

pub fn run<T:Send>(title: impl Into<String>, app: Box<dyn std::ops::FnOnce(&Context,&mut RecordingCommandBuffer) -> Result<Box<dyn App<T>>, Error>>) -> Result<(), Error> {
	let library = VulkanLibrary::new()?;
	let event_loop = EventLoop::new()?;
	let enabled_extensions = InstanceExtensions{ext_debug_utils: true, ..Surface::required_extensions(&event_loop)?};
	let enabled_layers = if false { vec!["VK_LAYER_KHRONOS_validation".to_owned()] } else { vec![] };
	let instance = Instance::new(library, InstanceCreateInfo{enabled_extensions, enabled_layers, ..default()})?;
	let ref window = Arc::new( WindowBuilder::new()
		.with_fullscreen(Some(Fullscreen::Borderless(event_loop.available_monitors().skip(0).next())))
		.with_title(title)
		.build(&event_loop)?);
	let surface = Surface::from_window(instance.clone(), window.clone())?;
	let enabled_extensions = DeviceExtensions{khr_swapchain: true, ..default()};
 	let (physical_device, queue_family_index) = instance.enumerate_physical_devices()?.find_map(|p| {
		((p.properties().device_type == PhysicalDeviceType::DiscreteGpu || p.properties().device_type == PhysicalDeviceType::IntegratedGpu) && p.supported_extensions().contains(&enabled_extensions))
			.then_some(())?;
		let (i, _) = p.queue_family_properties().iter().enumerate()
			.find(|&(i, q)| q.queue_flags.intersects(QueueFlags::GRAPHICS) && p.surface_support(i as u32, &surface).unwrap_or(false))?;
		Some((p, i as u32))
	}).unwrap();
	let (device, mut queues) = Device::new(physical_device, DeviceCreateInfo{
		enabled_extensions,
		queue_create_infos: vec![QueueCreateInfo{queue_family_index, ..default()}],
		enabled_features: DeviceFeatures{dynamic_rendering: true, dynamic_rendering_unused_attachments: true, ..default()},
		..default()
	})?;
 	let queue = queues.next().unwrap();
	let format = Format::B8G8R8A8_SRGB;
	let surface_capabilities = device.physical_device().surface_capabilities(&surface, default()).unwrap();
	let (mut swapchain, mut targets) = Swapchain::new(device.clone(), surface, SwapchainCreateInfo{
		min_image_count: surface_capabilities.min_image_count.max(2),
		image_format: format,
		image_extent: window.inner_size().into(),
		image_usage: ImageUsage::COLOR_ATTACHMENT|ImageUsage::TRANSFER_SRC,
		..default()
	})?;
	let ref mut context = Context{
		memory_allocator: Arc::new(StandardMemoryAllocator::new_default(device.clone())),
		command_buffer_allocator: Arc::new(StandardCommandBufferAllocator::new(device.clone(), default())),
		descriptor_set_allocator: Arc::new(StandardDescriptorSetAllocator::new(device.clone(), default())),
		device, queue, format,
	};
	let mut commands = RecordingCommandBuffer::new(context.command_buffer_allocator.clone(), context.queue.queue_family_index(),
					CommandBufferLevel::Primary, CommandBufferBeginInfo{usage: CommandBufferUsage::OneTimeSubmit, ..default()})?;
	let mut app = app(context, &mut commands)?;
	let mut pointer_position = default();
	let mut previous_frame_end : Option<FenceSignalFuture<Box<dyn GpuFuture>>> = Some(
		(Box::new(commands.end()?.execute(context.queue.clone())?) as Box<dyn GpuFuture>).then_signal_fence_and_flush()? );
	let mut recreate_swapchain = false;
	event_loop.run(move |event, target| (||->Result<(), Error>{
		let size = <[_;2]>::from(window.inner_size()).into();
		if let Event::WindowEvent{event, ..} = &event { match event {
			Resized(_) => recreate_swapchain = true,
			RedrawRequested => {
				if let Some(previous_frame_end) = previous_frame_end.as_mut() { previous_frame_end.cleanup_finished(); }
				if recreate_swapchain {
					(swapchain, targets) = swapchain.recreate(SwapchainCreateInfo{image_extent: window.inner_size().into(), ..swapchain.create_info()})?;
					recreate_swapchain = false;
				}

				let (target_index, suboptimal, acquire_future) = match acquire_next_image(swapchain.clone(), None).map_err(Validated::unwrap) {
					Ok(r) => r,
					Err(VulkanError::OutOfDate) => {recreate_swapchain = true; return Ok(());}
					Err(e) => panic!("{e}"),
				};
				if suboptimal { recreate_swapchain = true; }
				let ref target = targets[target_index as usize];
				/*if depth.as_ref().filter(|depth| depth.extent() == target.extent()).is_none() {
					depth = Some(Image::new(context.memory_allocator.clone(), ImageCreateInfo{format: Format::/*D16_UNORM*/D32_SFLOAT/*FIXME*/, extent: target.extent(),
					usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT, ..default()}, default())?);
				}*/

				let mut commands = RecordingCommandBuffer::new(context.command_buffer_allocator.clone(), context.queue.queue_family_index(),
					CommandBufferLevel::Primary, CommandBufferBeginInfo{usage: CommandBufferUsage::OneTimeSubmit, ..default()})?;
				app.render(&context, &mut commands, None, ImageView::new_default(target.clone())?, None)?;

				previous_frame_end =
					(Box::new(previous_frame_end.take().map(|fence_signal| fence_signal.boxed()).unwrap_or_else(|| now(context.device.clone()).boxed())
					.join(acquire_future)
					.then_execute(context.queue.clone(), commands.end()?)?
					.then_swapchain_present(context.queue.clone(), SwapchainPresentInfo::swapchain_image_index(swapchain.clone(), target_index)))
					as Box<dyn GpuFuture>).then_signal_fence_and_flush()
				 	.map_err(Validated::unwrap)
				 	.inspect_err(|e| if let VulkanError::OutOfDate = e { recreate_swapchain = true; })
					.ok();
			}
			CloseRequested|KeyboardInput{event:KeyEvent{logical_key:Named(Escape), state:Pressed, ..},..} => target.exit(),
			KeyboardInput{event: KeyEvent{logical_key: Character(key), state: Pressed, ..}, ..} => {
				let key = key.as_str().chars().next().unwrap();
				if key=='q' { target.exit(); }
			}
			CursorMoved{position, ..} => pointer_position = xy{x: position.x as _, y: position.y as _},
			CursorEntered{..} =>	window.focus_window(),
			_ => {}
		}}
		if app.event(size, pointer_position, 0, event) || recreate_swapchain { window.request_redraw() };
		Ok(())
	})().unwrap()).unwrap();
	Ok(())
}
