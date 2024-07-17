pub fn default<T: Default>() -> T { Default::default() }
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T=(), E=Error> = std::result::Result<T, E>;
pub use fehler::throws;
use {std::sync::Arc, vulkano::{device::{Device, Queue}, memory::allocator::StandardMemoryAllocator, command_buffer::allocator::StandardCommandBufferAllocator,  format::Format, descriptor_set::allocator::StandardDescriptorSetAllocator}};

#[derive(Clone)] pub struct Context {
	pub device: Arc<Device>,
	pub queue: Arc<Queue>,
	pub memory_allocator: Arc<StandardMemoryAllocator>,
	pub command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
	pub descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
	pub format: Format,
}

/*use vulkano::{memory::allocator::{AllocationCreateInfo, MemoryTypeFilter}, buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer, subbuffer::BufferContents}};
#[throws] pub fn buffer<T: BufferContents>(memory_allocator: Arc<StandardMemoryAllocator>, usage: BufferUsage, len: u32, iter: impl IntoIterator<Item=T>) -> Subbuffer<[T]> {
	let buffer = Buffer::new_slice::<T>(
		memory_allocator.clone(),
		BufferCreateInfo{usage, ..default()},
		AllocationCreateInfo{memory_type_filter: MemoryTypeFilter::PREFER_DEVICE|MemoryTypeFilter::HOST_SEQUENTIAL_WRITE, ..default()},
		len as u64
	)?;
	{
		let mut write_guard = buffer.write()?;
		for (o, i) in write_guard.iter_mut().zip(iter.into_iter()) { *o = i; }
	}
	buffer
}*/