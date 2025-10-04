struct Uniforms { altitude : f32 }
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var linear: sampler;

struct VertexOutput {
	@builtin(position) position: vec4<f32>,
	@location(0) texture_coordinates: vec2<f32>,
}

const position = array(vec2(-1., 1.), vec2(-1., -3.), vec2(3., 1.));
const texture_coordinates  = array(vec2(0., 0.), vec2(0., 2.), vec2(2., 0.));

@vertex fn vertex(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
	return VertexOutput(vec4(position[vertex_index], 0., 1.), vec2(texture_coordinates[vertex_index]));
}

@fragment fn fragment(vertex: VertexOutput) -> @location(0) vec4<f32> {
	let view_direction = vec3(0., 0., -1.);
	const PI = radians(180.0);
	let view_fov_width = PI/3.;
	let view_width_scale = 2.*tan(view_fov_width/2.);
	let view_height_scale = view_width_scale*2160./3840.;
	let view_right = normalize(cross(view_direction, vec3(0., 1., 0.)));
	let view_up = normalize(cross(view_right, view_direction));
	//let vertex_position = vertex.texture_coordinates * 2. - 1.; // FIXME: vertex.position is weird
	let vertex_position = vertex.texture_coordinates * 2. - vec2(1., 2.); // Horizon on bottom edge
	let ray_direction = normalize(view_direction + vertex_position.x*view_width_scale*view_right - vertex_position.y*view_height_scale*view_up);
	let up = vec3(0., 1., 0.);
	const ground_radius_Mm : f32 = 6.360;
	const view_position_y : f32 = ground_radius_Mm + 0.0002; // 200m above the ground.
	let horizon_angle = acos(sqrt(view_position_y * view_position_y - ground_radius_Mm * ground_radius_Mm) / view_position_y);
	let altitude_angle = horizon_angle - acos(dot(ray_direction, up)); // Between -PI/2 and PI/2
	let sun_direction = vec3(0., sin(uniforms.altitude), -cos(uniforms.altitude));
	let right = cross(sun_direction, up);
	let forward = cross(up, right);
	let projected_direction = normalize(ray_direction - dot(ray_direction, up)*up);
	let sin_theta = dot(projected_direction, right);
	let cos_theta = dot(projected_direction, forward);
	let azimuth_angle = atan2(sin_theta, cos_theta) + PI;
	let v = 0.5 + 0.5*sign(altitude_angle)*sqrt(abs(altitude_angle)*2.0/PI); // Non-linear mapping of altitude angle. See Section 5.3 of the paper.
	return textureSample(image, linear, vec2(azimuth_angle / (2.*PI), v));
}
