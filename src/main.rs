#![feature(slice_from_ptr_range,iter_next_chunk)]#![allow(non_snake_case,non_upper_case_globals)]
use ui::{Result, run, Widget, size, int2, vulkan, shader, EventContext, Event};
use vulkan::{Context, Commands, Arc, Image as GPUImage, image as upload, PrimitiveTopology, ImageView, WriteDescriptorSet, linear};
use image::{Image, rgb, rgbf, bilinear_sample_wrap_x_clamp_y, rgba8};
use {std::f32::consts::PI, num::{sq, sqrt, sin, cos, acos, abs}, vector::{xy, vec2, xyz, vec3, normalize, dot, norm}};
//use {num::{tan, atan, sign}, vector::cross};
fn exp(x: f32) -> f32 { f32::exp(x) }
fn pow(x: f32, k: f32) -> f32 { f32::powf(x, k) }
//fn min(a: f32, b: f32) -> f32 { f32::min(a, b) }
fn max(a: f32, b: f32) -> f32 { f32::max(a, b) }
fn mix(x: f32, y: f32, a: f32) -> f32 { x*(1.-a)+y*a }
shader!{sky}

// https://www.shadertoy.com/view/slSXRW
fn spherical(theta : f32, phi: f32) -> vec3 { xyz{x: sin(phi)*sin(theta), y: cos(phi), z: sin(phi)*cos(theta)} }
fn intersect_ray_sphere(origin: vec3, direction : vec3, radius: f32) -> f32 {
	let b = dot(origin, direction);
	let c = dot(origin, origin) - radius*radius;
	if c > 0. && b > 0. { return -1.; }
	let discriminant = b*b - c;
	if discriminant < 0. { return -1.; }
	if discriminant > b*b { -b + sqrt(discriminant) } else { -b - sqrt(discriminant) }
}

const ground_radius_Mm : f32 = 6.360;
const atmosphere_radius_Mm : f32 = 6.460;
const ground_albedo : rgbf = rgb{r: 0.3, g: 0.3, b: 0.3};

fn lookup(ref LUT: Image<&[rgbf]>, position: vec3 , sunDir: vec3) -> rgbf {
	let height = norm(position);
	let up = position / height;
	let cos_zenith = dot(sunDir, up);
	let uv = xy{
		x: LUT.size.x as f32*(1./2. + cos_zenith/2.)/*.clamp(0., 1.)*/,
		y: LUT.size.y as f32*((height - ground_radius_Mm)/(atmosphere_radius_Mm - ground_radius_Mm))/*.clamp(0., 1.)*/
	};
	bilinear_sample_wrap_x_clamp_y(LUT, uv)
}

fn scattering_extinction(position: vec3) -> (rgbf, f32, rgbf) {
	let altitude_km = (norm(position)-ground_radius_Mm)*1000.0;
	let rayleigh_density = exp(-altitude_km/8.0);
	let mie_density = exp(-altitude_km/1.2);
	const rayleigh_scattering_base : rgbf = rgb{r: 5.802, g: 13.558, b: 33.1};
	let rayleigh_scattering = rayleigh_density*rayleigh_scattering_base;
	const mie_scattering_base : f32 = 3.996;
	const mie_absorption_base : f32 = 4.4;
	let mie_scattering = mie_scattering_base*mie_density;
	let mie_absorption = mie_absorption_base*mie_density;
	const ozone_absorption_base : rgbf = rgb{r: 0.650, g: 1.881, b: 0.085};
	let ozone_absorption = max(0.0, 1.0 - abs(altitude_km-25.0)/15.0)*ozone_absorption_base;
	let extinction = rayleigh_scattering + rgbf::from(mie_scattering) + rgbf::from(mie_absorption) + ozone_absorption;
	(rayleigh_scattering, mie_scattering, extinction)
}

fn transmittance(pos: vec3, sunDir: vec3) -> rgbf {
	if intersect_ray_sphere(pos, sunDir, ground_radius_Mm) >= 0. { return rgbf::from(0.0); }
	let atmosphere_t = intersect_ray_sphere(pos, sunDir, atmosphere_radius_Mm);
	let mut t = 0.0;
	let mut transmittance = rgbf::from(1.0);
	const steps : usize = 40;
	for i in 0..steps {
		let newT = ((i as f32 + 0.3)/steps as f32)*atmosphere_t;
		let dt = newT - t;
		t = newT;
		let (_, _, extinction) = scattering_extinction(pos + t*sunDir);
		transmittance *= extinction.map(|e| exp(-dt*e));
	}
	transmittance
}

const multiple_scattering_steps : usize = 20;
const sqrt_samples : usize = 8;

fn mie_phase(cos_theta: f32) -> f32 {
	const g : f32 = 0.8;
	3./(8.*PI)*(1.-g*g)*(1.+cos_theta*cos_theta)/((2.+g*g)*pow(1. + g*g - 2.*g*cos_theta, 3./2.))
}
fn rayleigh_phase(cos_theta: f32) -> f32 { 3./(16.*PI)*(1.+cos_theta*cos_theta) }

type Tables = (Image<Box<[rgbf]>>, Image<Box<[rgbf]>>);
fn tables() -> Tables {
	let transmittance_size = xy{x: 256, y: 64};
	let transmittance_table = Image::from_xy(transmittance_size, |xy{x,y}| {
		let u = x as f32/transmittance_size.x as f32;
		let v = y as f32/transmittance_size.y as f32;
		let cos_theta = 2.*u - 1.;
		let height = mix(ground_radius_Mm, atmosphere_radius_Mm, v);
		transmittance(xyz{x: 0., y: height, z: 0.}, normalize(xyz{x: 0., y: cos_theta, z: -(1.-sq(cos_theta))}))
	});

	let multiple_scattering_size = xy{x: 32, y: 32};
	let multiple_scattering_table = Image::from_xy(multiple_scattering_size, |xy{x,y}| {
		let u = x as f32/multiple_scattering_size.x as f32;
		let v = y as f32/multiple_scattering_size.y as f32;
		let cos_theta = 2.*u - 1.;
		let height = mix(ground_radius_Mm, atmosphere_radius_Mm, v);
		let position = xyz{x: 0., y: height, z: 0.};
		let sun_direction = normalize(xyz{x: 0., y: cos_theta, z: -(1.-sq(cos_theta))});
		let mut luminance_total = rgb::from(0.0);
		let mut factor_multiple_scattering = rgb::from(0.0);
		const rcp_samples: f32 = 1.0/(sqrt_samples*sqrt_samples) as f32;
		for i in 0..sqrt_samples { for j in 0..sqrt_samples {
			// This integral is symmetric about theta = 0 (or theta = PI), so we only need to integrate from zero to PI, not zero to 2*PI.
			let theta = PI * (i as f32 + 1./2.) / sqrt_samples as f32;
			let phi = acos(1. - 2.*(j as f32 + 1./2.) / sqrt_samples as f32);
			let ray_direction = spherical(theta, phi);
			let atmosphere_t = intersect_ray_sphere(position, ray_direction, atmosphere_radius_Mm);
			let ground_t = intersect_ray_sphere(position, ray_direction, ground_radius_Mm);
			let max_t = if ground_t >= 0. { ground_t } else { atmosphere_t };
			let cos_theta = dot(ray_direction, sun_direction);
			let mie_phase = mie_phase(cos_theta);
			let rayleigh_phase = rayleigh_phase(-cos_theta);

			let mut luminance = rgb::from(0.0);
			let mut luminance_factor = rgb::from(0.0);
			let mut transmittance = rgb::from(1.0);
			let mut t = 0.;
			for i in 0..multiple_scattering_steps {
				let new_t = ((i as f32+ 0.3)/multiple_scattering_steps as f32)*max_t;
				let dt = new_t - t;
				t = new_t;
				let new_position = position + t*ray_direction;
				let (rayleigh_scattering, mie_scattering, extinction) = scattering_extinction(new_position);
				let sample_transmittance = extinction.map(|e| exp(-dt*e));
				let scattering_no_phase = rayleigh_scattering + rgb::from(mie_scattering);
				let scattering_factor = (scattering_no_phase - scattering_no_phase * sample_transmittance) / extinction;
				luminance_factor += transmittance*scattering_factor;
				// This is slightly different from the paper, but I think the paper has a mistake?
				// In equation (6), I think S(x,w_s) should be S(x-tv,w_s).
				let sun_transmittance = lookup(transmittance_table.as_ref(), new_position, sun_direction);
				let rayleigh_inscattering = rayleigh_phase*rayleigh_scattering;
				let mie_inscattering = mie_scattering*mie_phase;
				let inscattering = (rayleigh_inscattering + rgb::from(mie_inscattering))*sun_transmittance;
				// Integrated scattering within path segment.
				let scattering_integral = (inscattering - inscattering * sample_transmittance) / extinction;
				luminance += scattering_integral*transmittance;
				transmittance *= sample_transmittance;
			}

			if ground_t > 0.0 {
				if dot(position, sun_direction) > 0.0 {
					let hitPos = ground_radius_Mm*normalize(position + ground_t*ray_direction);
					luminance += transmittance*ground_albedo*lookup(transmittance_table.as_ref(), hitPos, sun_direction);
				}
			}
			factor_multiple_scattering += rcp_samples * luminance_factor;
			luminance_total += rcp_samples * luminance;
		}}
    	luminance_total  / (rgb::from(1.) - factor_multiple_scattering) // psi
	});
	(transmittance_table, multiple_scattering_table)
}

fn sky((transmittance_table, multiple_scattering): &Tables, altitude: f32, context: &Context, commands: &mut Commands) -> Result<Arc<GPUImage>> {
	const view_position_y : f32 = ground_radius_Mm + 2e-6; // 2m above the ground.
	const view_position : vec3 = xyz{x: 0., y: view_position_y, z: 0.};
	let sun_direction = xyz{x: 0., y: sin(altitude), z: -cos(altitude)};
	
	let sky_size = xy{x: 200, y: 100};
	let sky_table = Image::from_xy(sky_size, |xy| {
		let xy{x: u, y: v} = vec2::from(xy)/vec2::from(sky_size);
    	let azimuth_angle = (u - 0.5)*2.0*PI;
     	let v_parametrization = if  v < 1./2. { -sq(1. - 2.*v) } else { sq(v*2. - 1.) };
		let horizon_angle = acos(sqrt(view_position_y * view_position_y - ground_radius_Mm * ground_radius_Mm) / view_position_y) - PI/2.;
		let altitude_angle = v_parametrization*PI/2. - horizon_angle;
		let cos_altitude = cos(altitude_angle);
		let ray_direction = xyz{x: cos_altitude*sin(azimuth_angle), y: sin(altitude_angle), z: -cos_altitude*cos(azimuth_angle)};
		let atmosphere_t = intersect_ray_sphere(view_position, ray_direction, atmosphere_radius_Mm);
	    let ground_t = intersect_ray_sphere(view_position, ray_direction, ground_radius_Mm);
	    let max_t = if ground_t < 0. { atmosphere_t } else { ground_t };
		let cos_theta = dot(ray_direction, sun_direction);
		let mie_phase = mie_phase(cos_theta);
		let rayleigh_phase = rayleigh_phase(-cos_theta);
	    let mut luminance = rgb::from(0.0);
	    let mut transmittance = rgb::from(1.0);
	    let mut t = 0.0;
		const steps : i32 = 32;
		for i in 0..steps {
			let new_t = ((i as f32 + 0.3)/steps as f32)*max_t;
			let dt = new_t - t;
			t = new_t;
			let new_position = view_position + t*ray_direction;
			//assert!(norm(newPos) >= groundRadiusMM);
			let (rayleigh_scattering, mie_scattering, extinction) = scattering_extinction(new_position);
			let sample_transmittance = extinction.map(|e| exp(-dt*e));
			let sun_transmittance = lookup(transmittance_table.as_ref(), new_position, sun_direction);
			let psi_multiple_scattering = lookup(multiple_scattering.as_ref(), new_position, sun_direction);
			let rayleigh_inscattering = rayleigh_scattering*(rayleigh_phase*sun_transmittance + psi_multiple_scattering);
			let mie_inscattering = mie_scattering*(mie_phase*sun_transmittance + psi_multiple_scattering);
			let inscattering = rayleigh_inscattering + mie_inscattering;
			let scatteringIntegral = (inscattering - sample_transmittance * inscattering) / extinction;
			luminance += scatteringIntegral*transmittance;
			transmittance *= sample_transmittance;
		}
		luminance
	});
	let ref oetf = image::sRGB8_OETF12; // reversed by texture lookup in fragment shader
    upload(context, commands, sky_table.map(|v| rgba8::from(image::oetf8_12_rgb(oetf, v))).as_ref())
}

struct App {
	pass: sky::Pass,
	tables: Tables,
	altitude: f32,
	last_altitude: f32,
	sky: Arc<GPUImage>,
	yaw: f32,
}
impl App {
	fn new(context: &Context, commands: &mut Commands) -> Result<Self> {
		let tables = tables();
		Ok(Self{
			pass: sky::Pass::new(context, false, PrimitiveTopology::TriangleList, false)?,
			altitude: 0., last_altitude: 0.,
			sky: sky(&tables, 0., context, commands)?,
			tables,
			yaw: 0.
		})
	}
}
impl Widget for App {
fn paint(&mut self, context: &Context, commands: &mut Commands, target: Arc<ImageView>, _: size, _: int2) -> Result<()> {
	let Self{pass, tables, altitude, last_altitude, sky, yaw} = self;
	if altitude != last_altitude {
		*sky = self::sky(&tables, *altitude, context, commands)?;
		*last_altitude = *altitude;
	}
	pass.begin_rendering(context, commands, target.clone(), None, true, &sky::Uniforms{altitude: *altitude, yaw: *yaw}, &[
		WriteDescriptorSet::image_view(1, ImageView::new_default(&sky)?),
		WriteDescriptorSet::sampler(2, linear(context)),
	])?;
	unsafe{commands.draw(3, 1, 0, 0)}?;
	commands.end_rendering()?;
	Ok(())
}
fn event(&mut self, _: &Context, _: &mut Commands, size: size, _: &mut EventContext, event: &Event) -> Result<bool> { Ok(match event {
	Event::Motion{position, ..} => {
		self.yaw = (position.x as f32 / size.x as f32) * 2.*PI;
		self.altitude = (1. - position.y as f32 / size.y as f32) * PI/2.;
		true
	}
	_ => false,
})}
}

fn main() -> Result { run("pathtrace", Box::new(|context, commands| Ok(Box::new(App::new(context, commands)?)))) }
