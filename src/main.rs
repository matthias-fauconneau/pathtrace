#![feature(slice_from_ptr_range,iter_next_chunk)]#![allow(non_snake_case,non_upper_case_globals)]
use ui::{Result, run, Widget, size, int2, vulkan, shader};
use vulkan::{Context, Commands, Arc, image as upload, PrimitiveTopology, ImageView, WriteDescriptorSet, linear};
use image::{Image, rgb, rgbf, bilinear_sample_wrap_x_clamp_y, sRGB8_OETF12, oetf8_12_rgb, rgba8};
use {std::f32::consts::PI, num::{sq, sqrt, sin, cos, tan, acos, atan, sign, abs}, vector::{xy, vec2, xyz, vec3, normalize, cross, dot, norm}};
fn exp(x: f32) -> f32 { f32::exp(x) }
fn pow(x: f32, k: f32) -> f32 { f32::powf(x, k) }
fn min(a: f32, b: f32) -> f32 { f32::min(a, b) }
fn max(a: f32, b: f32) -> f32 { f32::max(a, b) }
//fn clamp(x: f32, min: f32, max: f32) -> f32 { f32::clamp(x, min, max) }
fn mix(x: f32, y: f32, a: f32) -> f32 { x*(1.-a)+y*a }
#[track_caller] fn check(v: rgbf) -> rgbf { for c in v.into_iter() { assert!(c.is_finite()); } v }
shader!{view}

struct App {
	pass: view::Pass,
}
impl App {
	fn new(context: &Context, _: &mut Commands) -> Result<Self> {
		Ok(Self{pass: view::Pass::new(context, false, PrimitiveTopology::TriangleList, false)?})
	}
}

// https://www.shadertoy.com/view/slSXRW
fn getSphericalDir(theta : f32, phi: f32) -> vec3 { xyz{x: sin(phi)*sin(theta), y: cos(phi), z: sin(phi)*cos(theta)} }
fn rayIntersectSphere(ro: vec3, rd : vec3, radius: f32) -> f32 {
	let b = dot(ro, rd);
	let c = dot(ro, ro) - radius*radius;
	if c > 0. && b > 0. { return -1.; }
	let discriminant = b*b - c;
	if discriminant < 0. { return -1.; }
	if discriminant > b*b { -b + sqrt(discriminant) } else { -b - sqrt(discriminant) }
}

fn getValFromLUT(ref LUT: Image<&[rgbf]>, pos: vec3 , sunDir: vec3) -> rgbf {
	let height = norm(pos);
	let up = pos / height;
	let sunCosZenithAngle = dot(sunDir, up);
	let uv = xy{
		x: LUT.size.x as f32*max(0., min(1., 1./2. + sunCosZenithAngle/2.)),
		y: LUT.size.y as f32*max(0., min(1., (height - groundRadiusMM)/(atmosphereRadiusMM - groundRadiusMM)))
	};
	bilinear_sample_wrap_x_clamp_y(LUT, uv)
}

const groundRadiusMM : f32 = 6.360;
const atmosphereRadiusMM : f32 = 6.460;
const groundAlbedo : rgbf = rgb{r: 0.3, g: 0.3, b: 0.3};

fn getScatteringValues(pos: vec3) -> (rgbf, f32, rgbf) {
	let altitudeKM = (norm(pos)-groundRadiusMM)*1000.0;
	//assert!(altitudeKM >= 0., "{pos:?} {} {altitudeKM}", norm(pos));
	let rayleighDensity = exp(-altitudeKM/8.0);
	let mieDensity = exp(-altitudeKM/1.2);
	const rayleighScatteringBase : rgbf = rgb{r: 5.802, g: 13.558, b: 33.1};
	let rayleighScattering = rayleighDensity*rayleighScatteringBase;
	const mieScatteringBase : f32 = 3.996;
	const mieAbsorptionBase : f32 = 4.4;
	let mieScattering = mieScatteringBase*mieDensity;
	let mieAbsorption = mieAbsorptionBase*mieDensity;
	const ozoneAbsorptionBase : rgbf = rgb{r: 0.650, g: 1.881, b: 0.085};
	let ozoneAbsorption = max(0.0, 1.0 - abs(altitudeKM-25.0)/15.0)*ozoneAbsorptionBase;
	let extinction = rayleighScattering + rgbf::from(mieScattering) + rgbf::from(mieAbsorption) + ozoneAbsorption;
	(rayleighScattering, mieScattering, extinction)
}

fn getSunTransmittance(pos: vec3, sunDir: vec3) -> rgbf {
	if rayIntersectSphere(pos, sunDir, groundRadiusMM) >= 0. { return rgbf::from(0.0); }
	let atmoDist = rayIntersectSphere(pos, sunDir, atmosphereRadiusMM);
	assert!(atmoDist > 0., "{atmoDist} {pos:?} {sunDir:?} {atmosphereRadiusMM}");
	let mut t = 0.0;
	let mut transmittance = rgbf::from(1.0);
	const sunTransmittanceSteps : usize = 40;
	for i in 0..sunTransmittanceSteps {
		let newT = ((i as f32 + 0.3)/sunTransmittanceSteps as f32)*atmoDist;
		let dt = newT - t;
		t = newT;
		let newPos = pos + t*sunDir;
		let (_, _, extinction) = getScatteringValues(newPos);
		transmittance *= extinction.map(|e| exp(-dt*e));
	}
	transmittance
}

const mulScattSteps : usize = 20;
const sqrtSamples : usize = 8;

fn getMiePhase(cosTheta: f32) -> f32 {
	const g : f32 = 0.8;
	const scale : f32 = 3./(8.*PI);
	let num = (1.-g*g)*(1.+cosTheta*cosTheta);
	let denom = (2.+g*g)*pow(1. + g*g - 2.*g*cosTheta, 3./2.);
	scale*num/denom
}
fn getRayleighPhase(cosTheta: f32) -> f32 {
	const k : f32 = 3.0/(16.0*PI);
   	k*(1.0+cosTheta*cosTheta)
}

fn getMulScattValues(transmissionLUT: Image<&[rgbf]>, pos: vec3, sunDir: vec3) -> (rgbf, rgbf) {
	let mut lumTotal = rgb::from(0.0);
	let mut fms = rgb::from(0.0);
	const invSamples: f32 = 1.0/(sqrtSamples*sqrtSamples) as f32;
	for i in 0..sqrtSamples { for j in 0..sqrtSamples {
		// This integral is symmetric about theta = 0 (or theta = PI), so we only need to integrate from zero to PI, not zero to 2*PI.
		let theta = PI * (i as f32 + 1./2.) / sqrtSamples as f32;
		let phi = acos(1. - 2.*(j as f32 + 1./2.) / sqrtSamples as f32);
		let rayDir = getSphericalDir(theta, phi);
		let atmoDist = rayIntersectSphere(pos, rayDir, atmosphereRadiusMM);
		let groundDist = rayIntersectSphere(pos, rayDir, groundRadiusMM);
		let tMax = if groundDist > 0. { groundDist } else { atmoDist };
		let cosTheta = dot(rayDir, sunDir);
		let miePhaseValue = getMiePhase(cosTheta);
		let rayleighPhaseValue = getRayleighPhase(-cosTheta);

		let mut lum = rgb::from(0.0);
		let mut lumFactor = rgb::from(0.0);
		let mut transmittance = rgb::from(1.0);
		let mut t = 0.;
		for stepI in 0..mulScattSteps {
			let newT = ((stepI as f32+ 0.3)/mulScattSteps as f32)*tMax;
			let dt = newT - t;
			t = newT;
			let newPos = pos + t*rayDir;
			let (rayleighScattering, mieScattering, extinction) = getScatteringValues(newPos);
			let sampleTransmittance = extinction.map(|e| exp(-dt*e));
			// Integrate within each segment.
			let scatteringNoPhase = rayleighScattering + rgb::from(mieScattering);
			let scatteringF = (scatteringNoPhase - scatteringNoPhase * sampleTransmittance) / extinction;
			lumFactor += transmittance*scatteringF;
			// This is slightly different from the paper, but I think the paper has a mistake?
			// In equation (6), I think S(x,w_s) should be S(x-tv,w_s).
			let sunTransmittance = getValFromLUT(transmissionLUT.as_ref(), newPos, sunDir);
			let rayleighInScattering = rayleighPhaseValue*rayleighScattering;
			let mieInScattering = mieScattering*miePhaseValue;
			let inScattering = (rayleighInScattering + rgb::from(mieInScattering))*sunTransmittance;
			// Integrated scattering within path segment.
			let scatteringIntegral = (inScattering - inScattering * sampleTransmittance) / extinction;
			lum += scatteringIntegral*transmittance;
			transmittance *= sampleTransmittance;
		}

		if groundDist > 0.0 {
			if dot(pos, sunDir) > 0.0 {
				let hitPos = groundRadiusMM*normalize(pos + groundDist*rayDir);
				lum += transmittance*groundAlbedo*getValFromLUT(transmissionLUT.as_ref(), hitPos, sunDir);
			}
		}
		fms += invSamples*lumFactor;
		lumTotal += invSamples*lum;
	}}
	(lumTotal, fms)
}

impl Widget for App {
fn paint(&mut self, context: &Context, commands: &mut Commands, target: Arc<ImageView>, size: size, _: int2) -> Result<()> {
	let Self{pass, ..} = self;

	let tLUTRes = xy{x: 256, y: 64};
	let transmittanceLUT = Image::from_xy(tLUTRes, |fragCoord| {
		let u = fragCoord.x as f32/tLUTRes.x as f32;
		let v = fragCoord.y as f32/tLUTRes.y as f32;
		let sunCosTheta = 2.*u - 1.;
		let sunTheta = acos(sunCosTheta);
		let height = mix(groundRadiusMM, atmosphereRadiusMM, v);
		let pos = xyz{x: 0., y: height, z: 0.};
		let sunDir = normalize(xyz{x: 0., y: sunCosTheta, z: -sin(sunTheta)});
		getSunTransmittance(pos, sunDir)
	});
	{
		let mut min = rgb::from(f32::INFINITY);
		let mut max = rgb::from(f32::NEG_INFINITY);
		for &v in &transmittanceLUT.data { 
			use vector::ComponentWiseMinMax;
			//for c in v { assert!(c.is_finite()); }
			min = min.component_wise_min(v);
			max = max.component_wise_max(v);
		}
		println!("transmittance {min:?} {max:?}");
	}
	
	let msLUTRes = xy{x: 32, y: 32};
	let multipleScatteringLUT = Image::from_xy(msLUTRes, |fragCoord| {
		let u = fragCoord.x as f32/msLUTRes.x as f32;
		let v = fragCoord.y as f32/msLUTRes.y as f32;
		let sunCosTheta = 2.*u - 1.;
		let sunTheta = acos(sunCosTheta);
		let height = mix(groundRadiusMM, atmosphereRadiusMM, v);
		let pos = xyz{x: 0., y: height, z: 0.};
		let sunDir = normalize(xyz{x: 0., y: sunCosTheta, z: -sin(sunTheta)});
		let (lum, f_ms) = getMulScattValues(transmittanceLUT.as_ref(), pos, sunDir);
    	lum  / (rgb::from(1.) - f_ms) // psi
	});
	/*{
		let mut min = rgb::from(f32::INFINITY);
		let mut max = rgb::from(f32::NEG_INFINITY);
		for &v in &multipleScatteringLUT.data { 
			use vector::ComponentWiseMinMax;
			min = min.component_wise_min(v);
			max = max.component_wise_max(v);
		}
		println!("MS: {min:?} {max:?}");
	}*/
	
	fn raymarchScattering(transmittanceLUT: Image<&[rgbf]>, multipleScatteringLUT: Image<&[rgbf]>, pos: vec3, rayDir: vec3, sunDir: vec3, tMax: f32) -> rgbf {
		let cosTheta = dot(rayDir, sunDir);
		let miePhaseValue = getMiePhase(cosTheta);
		let rayleighPhaseValue = getRayleighPhase(-cosTheta);
	    let mut lum = rgb::from(0.0);
	    let mut transmittance = rgb::from(1.0);
	    let mut t = 0.0;
		const numSteps : i32 = 32;
		for i in 0..numSteps {
			let newT = ((i as f32 + 0.3)/numSteps as f32)*tMax;
			let dt = newT - t;
			t = newT;
			let newPos = pos + t*rayDir;
			let (rayleighScattering, mieScattering, extinction) = getScatteringValues(newPos);
			let sampleTransmittance = extinction.map(|e| exp(-dt*e));
			let sunTransmittance = getValFromLUT(transmittanceLUT.as_ref(), newPos, sunDir);
			let psiMS = getValFromLUT(multipleScatteringLUT.as_ref(), newPos, sunDir);
			let rayleighInScattering = rayleighScattering*(rayleighPhaseValue*sunTransmittance + psiMS);
			let mieInScattering = mieScattering*(miePhaseValue*sunTransmittance + psiMS);
			let inScattering = rayleighInScattering + mieInScattering;
			let scatteringIntegral = (inScattering - sampleTransmittance * inScattering) / extinction;
	        lum += scatteringIntegral*transmittance;
	        transmittance *= sampleTransmittance;
	    }
	    lum
	}

	const viewPosY : f32 = groundRadiusMM + 0.0002; // 200m above the ground.
	const viewPos : vec3 = xyz{x: 0., y: viewPosY, z: 0.};

	let altitude = 0.;
	let sunDir = xyz{x: 0., y: sin(altitude), z: -cos(altitude)};

	let skyLUTRes = xy{x: 200, y: 100};
	let skyLUT = Image::from_xy(skyLUTRes, |xy| {
		let xy{x: u, y: v} = vec2::from(xy)/vec2::from(skyLUTRes);
    	let azimuthAngle = (u - 0.5)*2.0*PI;
     	// Non-linear mapping of altitude. See Section 5.3 of the paper.
		let adjV = if  v < 1./2. { -sq(1. - 2.*v) } else { sq(v*2. - 1.) };
		//let up = xyz{x: 0., y: 1., z: 0.};

		let horizonAngle = acos(sqrt(viewPosY * viewPosY - groundRadiusMM * groundRadiusMM) / viewPosY) - PI/2.;
		let altitudeAngle = adjV*PI/2. - horizonAngle;
		let cosAltitude = cos(altitudeAngle);
		let rayDir = xyz{x: cosAltitude*sin(azimuthAngle), y: sin(altitudeAngle), z: -cosAltitude*cos(azimuthAngle)};
		//let sunAltitude = (0.5*PI) - acos(dot(sunDir, up));

		let atmoDist = rayIntersectSphere(viewPos, rayDir, atmosphereRadiusMM);
	    let groundDist = rayIntersectSphere(viewPos, rayDir, groundRadiusMM);
	    let tMax = if groundDist < 0. { atmoDist } else { groundDist };
	    raymarchScattering(transmittanceLUT.as_ref(), multipleScatteringLUT.as_ref(), viewPos, rayDir, sunDir, tMax)
	});
	{
		let mut min = rgb::from(f32::INFINITY);
		let mut max = rgb::from(f32::NEG_INFINITY);
		for &v in &skyLUT.data { 
			use vector::ComponentWiseMinMax;
			//for c in v { assert!(c.is_finite()); }
			min = min.component_wise_min(v);
			max = max.component_wise_max(v);
		}
		println!("{min:?} {max:?}");
	}
	/*let image = Image::from_xy(size, |xy| {
		let camDir = xyz{x: 0., y: 0., z: -1.};
    	let camFOVWidth = PI/3.;
     	let camWidthScale = 2.*tan(camFOVWidth/2.);
      	let camHeightScale = camWidthScale*size.y as f32/size.x as f32;
        let camRight = normalize(cross(camDir, xyz{x: 0., y: 1., z: 0.}));
        let camUp = normalize(cross(camRight, camDir));
        let xy = 2. * (vec2::from(xy) / vec2::from(size)) - vec2::from(1.);
        let rayDir = normalize(camDir + xy.x*camWidthScale*camRight + xy.y*camHeightScale*camUp);

        fn getValFromSkyLUT(ref skyLUT: Image<&[rgbf]>, rayDir: vec3, sunDir: vec3) -> rgbf {
            let up = xyz{x: 0., y: 1., z: 0.};
            let horizonAngle = acos(sqrt(viewPosY * viewPosY - groundRadiusMM * groundRadiusMM) / viewPosY);
            let altitudeAngle = horizonAngle - acos(dot(rayDir, up)); // Between -PI/2 and PI/2
            let right = cross(sunDir, up);
            let forward = cross(up, right);
            let projectedDir = normalize(rayDir - dot(rayDir, up)*up);
            let sinTheta = dot(projectedDir, right);
            let cosTheta = dot(projectedDir, forward);
            let azimuthAngle = atan(sinTheta, cosTheta) + PI;
            let v = 0.5 + 0.5*sign(altitudeAngle)*sqrt(abs(altitudeAngle)*2.0/PI); // Non-linear mapping of altitude angle. See Section 5.3 of the paper.
            let uv = xy{x: azimuthAngle / (2.*PI), y: v};
            bilinear_sample_wrap_x_clamp_y(skyLUT, uv*vec2::from(skyLUT.size))
        }

        //rgb{r: rayDir.x, g: rayDir.y, b: rayDir.z}
        getValFromSkyLUT(skyLUT.as_ref(), rayDir, sunDir)
	});*/
	let ref oetf = sRGB8_OETF12;
	//let image = upload(context, commands, transmittanceLUT.map(|v| rgba8::from(oetf8_12_rgb(oetf, v.map(|c| c.clamp(0.,1.))))).as_ref())?;
	//let image = upload(context, commands, multipleScatteringLUT.map(|v| rgba8::from(oetf8_12_rgb(oetf, v.map(|c| c.clamp(0.,1.))))).as_ref())?;
	let image = upload(context, commands, skyLUT.map(|v| rgba8::from(oetf8_12_rgb(oetf, v.map(|c| c.clamp(0.,1.))))).as_ref())?;
	//let image = upload(context, commands, image.map(|v| rgba8::from(oetf8_12_rgb(oetf, v))).as_ref())?;
	pass.begin_rendering(context, commands, target.clone(), None, true, &view::Uniforms::empty(), &[
		WriteDescriptorSet::image_view(0, ImageView::new_default(&image)?),
		WriteDescriptorSet::sampler(1, linear(context)),
	])?;
	unsafe{commands.draw(3, 1, 0, 0)}?;
	commands.end_rendering()?;
	Ok(())
}
}

fn main() -> Result { run("pathtrace", Box::new(|context, commands| Ok(Box::new(App::new(context, commands)?)))) }
