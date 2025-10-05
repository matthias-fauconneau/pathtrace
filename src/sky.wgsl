const PI : f32 = radians(180.0);
struct Uniforms { altitude : f32, yaw: f32 }
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

const ground_radius_Mm : f32 = 6.360;
const view_altitude : f32 = 2.;
const view_position_y : f32 = ground_radius_Mm + view_altitude*1e-6; // 2m above the ground.

fn atmosphere_luminance(ray_direction: vec3f) -> vec3f {
	let up = vec3(0., 1., 0.);
	let horizon_angle = acos(sqrt(view_position_y * view_position_y - ground_radius_Mm * ground_radius_Mm) / view_position_y);
	let altitude_angle = horizon_angle - acos(dot(ray_direction, up)); // Between -PI/2 and PI/2
	let sun_direction = vec3(0., sin(uniforms.altitude), -cos(uniforms.altitude));
	let right = cross(sun_direction, up);
	let forward = cross(up, right);
	let projected_direction = normalize(ray_direction - dot(ray_direction, up)*up);
	let sin_theta = dot(projected_direction, right);
	let cos_theta = dot(projected_direction, forward);
	let azimuth_angle = atan2(sin_theta, cos_theta) + PI;
	let v = 1./2. + 1./2.*sign(altitude_angle)*sqrt(abs(altitude_angle)*2./PI); // Non-linear mapping of altitude angle. See Section 5.3 of the paper.
	return textureSample(image, linear, vec2(azimuth_angle / (2.*PI), v)).rgb;
}

fn intersect_ray_sphere(origin: vec3f, direction : vec3f, radius: f32) -> f32 {
	let b = dot(origin, direction);
	let c = dot(origin, origin) - radius*radius;
	if c > 0. && b > 0. { return -1.; }
	let discriminant = b*b - c;
	if discriminant < 0. { return -1.; }
	if discriminant > b*b { return -b + sqrt(discriminant); } else { return -b - sqrt(discriminant); };
}

// linearizes uv using a Hilbert curve; tile dimension = 2^N
fn hilbert_curve(uv: vec2u, N: u32) -> u32 {
	let C = 0xB4361E9Cu; // cost lookup
	let P = 0xEC7A9107u; // pattern lookup
	var c = 0u; // accumulated cost
	var p = 0u; // current pattern
	for(var i = N-1; i < N; i-=1) {
		let m = vec2u((uv.x >> i) & 1u, (uv.y >> i) & 1u); // local uv
		let n = m.x ^ (m.y << 1u);// linearized local uv
		let o = (p << 3u) ^ (n << 1u);// offset into lookup tables
		c += ((C >> o) & 3u) << (i << 1u);// accu cost (scaled by level)
		p = (P >> o) & 3u;// update pattern
	}
	return c;
}
// https://www.shadertoy.com/view/3lcczS http://www.jcgt.org/published/0009/04/01/
fn reverse_bits(x0: u32)  -> u32 {
	var x = x0;
	x = (((x & 0xaaaaaaaau) >> 1) | ((x & 0x55555555u) << 1));
	x = (((x & 0xccccccccu) >> 2) | ((x & 0x33333333u) << 2));
	x = (((x & 0xf0f0f0f0u) >> 4) | ((x & 0x0f0f0f0fu) << 4));
	x = (((x & 0xff00ff00u) >> 8) | ((x & 0x00ff00ffu) << 8));
    return ((x >> 16) | (x << 16));
}
fn lkhash(x0: u32, seed: u32)  -> u32 { // https://psychopath.io/post/2021_01_30_building_a_better_lk_hash
	var x = x0;
	x ^= x * 0x3d20adea;
	x += seed;
	x *= (seed >> 16) | 1;
	x ^= x * 0x05526c56;
	x ^= x * 0x53a22864;
	return x;
}
fn nested_uniform_scramble(x: u32, seed: u32) -> u32 { return reverse_bits(lkhash(reverse_bits(x), seed)); }
fn sobol_2d(index0: u32) -> vec2u { // https://www.shadertoy.com/view/3ldXzM
	var p = vec2u(0u);
	var d = vec2u(0x80000000u);
	for(var index=index0; index != 0u; index >>= 1u) {
		if((index & 1u) != 0u) { p ^= d; }
		d.x >>= 1u;  // 1st dimension Sobol matrix, is same as base 2 Van der Corput
		d.y ^= d.y >> 1u; // 2nd dimension Sobol matrix
	}
	return p;
}
fn shuffled_scrambled_sobol_2d(index: u32, seed0: u32) -> vec2u {
	var p = sobol_2d(nested_uniform_scramble(index, seed0));
	var seed = seed0 * 2891336453u + 1u;
	p.x = nested_uniform_scramble(p.x, seed);
    seed = seed * 2891336453u + 1u;
    p.y = nested_uniform_scramble(p.y, seed);
    return p;
}
fn float11(x: vec2u) -> vec2f { return vec2f(vec2i(x)) * (1./2147483648.); }

fn cosine(n: vec3f, vertex_index: u32) -> vec3f {
	let u = float11(shuffled_scrambled_sobol_2d(vertex_index, 0xCC925D21u));
	let r = sqrt(u.x);
	let theta = 2.*PI*u.y;
	let B = normalize(cross(n, vec3(0.,1.,1.)));
	let T = cross(B, n);
	return normalize(r * sin(theta) * B + sqrt(1.-u.x) * n + r * cos(theta) * T);
}

@fragment fn fragment(vertex: VertexOutput) -> @location(0) vec4f {
	let pixel_index = hilbert_curve(vec2u(vertex.position.xy), 11u);

	const r = 4.;
	const sphere_center = vec3(0., 1., 0.);
	let view_position = vec3(r*sin(uniforms.yaw), view_altitude, -r*cos(uniforms.yaw));
	let view_direction = normalize(sphere_center-view_position);

	let view_fov_width = PI/3.;
	let view_width_scale = 2.*tan(view_fov_width/2.);
	let view_height_scale = view_width_scale*2160./3840.;
	let view_right = normalize(cross(view_direction, vec3(0., 1., 0.)));
	let view_up = normalize(cross(view_right, view_direction));

	var luminance = vec3(0.);
	const samples = 32;
	for(var sample=0u; sample<samples; sample+=1) {
		let sample_index = pixel_index*samples+sample;
		var ray_origin = view_position;
		let vertex_position = vertex.texture_coordinates * 2. - 1.; // FIXME: vertex.position is weird
		var ray_direction = normalize(view_direction + vertex_position.x*view_width_scale*view_right - vertex_position.y*view_height_scale*view_up);
		var transmittance = vec3(1.);
		const bounces = 2;
		var bounce=0u;
		for(; bounce<bounces; bounce+=1) { // FIXME: russian roulette on path importance
			let vertex_index = sample_index*bounces+bounce;
			let t = intersect_ray_sphere(ray_origin-sphere_center, ray_direction, 1.);
			if t > 1e-5 {
				const sphere_albedo : f32 = 0.4;
				transmittance *= sphere_albedo;
				ray_origin = ray_origin + t * ray_direction;
				let normal = normalize(ray_origin-sphere_center);
				//ray_direction = normalize(ray_direction - 2.*dot(ray_direction, normal)*normal); // Specular
				ray_direction = cosine(normal, vertex_index);
				//let u = float11(shuffled_scrambled_sobol_2d(vertex_index, 0xCC925D21u));return vec4(u, 1., 1.);
			} else {
				//let t = intersect_ray_sphere(ray_origin*1e-6-vec3(0.,-ground_radius_Mm,0.), ray_direction, ground_radius_Mm);
				let t = - ray_origin.y / ray_direction.y;
				if t > 1e-5 {
					const ground_albedo : f32 = 0.4;
					transmittance *= ground_albedo;
					ray_origin = ray_origin + t * ray_direction;
					ray_direction = cosine(vec3(0.,1.,0.), vertex_index);
					//let u = float11(shuffled_scrambled_sobol_2d(vertex_index, 0xCC925D21u));					return vec4(u, 1., 1.);
				} else {
					luminance += transmittance * atmosphere_luminance(ray_direction);
					break;
				}
			}
		}
		if bounce == 0 { luminance *= f32(samples); break; } // Fast path for deterministic pixels (no diffuse bounces)
	}
	return vec4(16.*luminance/f32(samples), 1.);
}
