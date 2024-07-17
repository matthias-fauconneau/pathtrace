@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var linear_interpolation: sampler;

struct VertexOutput {
	@builtin(position) position: vec4<f32>,
	@location(1) texture_coordinates: vec2<f32>,
}

@vertex fn vertex(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
	let texture_coordinates = vec2(f32(vertex_index >> 1), f32(vertex_index & 1)) * 2.;
	return VertexOutput(vec4(texture_coordinates * vec2(2., -2.) + vec2(-1., 1.), 0., 1.), texture_coordinates);
}

@fragment fn fragment(vertex: VertexOutput) -> @location(0) vec4<f32> {
	return textureSample(image, linear_interpolation, vec2(vertex.texture_coordinates.x, vertex.texture_coordinates.y));
}
