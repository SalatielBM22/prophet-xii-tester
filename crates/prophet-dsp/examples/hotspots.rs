//! Isolated component wall-time profile, not a hardware cycle counter.
use prophet_dsp::{fx::solaris_reverb::*,vcf_cem3320::Cem3320Filter,vcf_moog::MoogLadderFilter};
use std::{hint::black_box,time::Instant};
fn bench(mut f:impl FnMut(f32)->f32)->f64 {let start=Instant::now();for i in 0..96000 {black_box(f(if i%1000==0{0.2}else{0.0}));}start.elapsed().as_secs_f64()*1e9/96000.}
fn main(){
 println!("component,ns_per_sample");
 let mut c=Cem3320Filter::new(48000.,1.);c.set_params(2000.,0.85,1.8);println!("curtis,{:.2}",bench(|x|c.process_sample(x)));
 let mut m=MoogLadderFilter::new(48000.,1.);m.set_params(2000.,0.85,1.8,0.5);println!("moog,{:.2}",bench(|x|m.process_sample(x)));
 for mode in [EnsembleMode::Off,EnsembleMode::MezzoPiano,EnsembleMode::Forte]{let mut r=SolarisCloudburstReverb::new(48000.);let p=SolarisReverbParams{ensemble_mode:mode,..Default::default()};println!("reverb-{mode:?},{:.2}",bench(|x|r.process_stereo(x,x,&p).0));}
}
