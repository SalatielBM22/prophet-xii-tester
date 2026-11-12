//! Reproductions for #1/#2. No source/UI artifacts are overwritten.
use prophet_dsp::fx::solaris_reverb::*;
fn isolated() -> SolarisReverbParams {SolarisReverbParams { wet_mix:1.,pre_delay_ms:0.,decay_sec:0.3,ensemble_mode:EnsembleMode::Off,shimmer_amount:0.,vowel_mix:0.,mod_depth:0.,..Default::default()}}
#[test]
fn zero_wet_must_not_suspend_a_stale_tail(){
 let mut r=SolarisCloudburstReverb::new(48000.);let mut p=isolated();
 for i in 0..4800 {r.process_stereo(if i==0{0.5}else{0.},0.,&p);}
 p.wet_mix=0.;for _ in 0..144000 {assert_eq!(r.process_stereo(0.,0.,&p),(0.,0.));}
 p.wet_mix=1.;let mut energy=0.;for _ in 0..9600 {let(l,rr)=r.process_stereo(0.,0.,&p);energy+=l*l+rr*rr;}
 assert!(energy<1e-12,"old tail resumed: {energy}");
}
#[test]
fn bypass_kill_spill_and_reenable_are_explicit(){
 for sr in [48000.,96000.] { for policy in [ReverbTailPolicy::Kill,ReverbTailPolicy::Spill] {
  let mut r=SolarisCloudburstReverb::new(sr);let mut p=isolated();p.bypass_tail=policy;
  for i in 0..(sr*0.1) as usize {r.process_stereo(if i==0{0.5}else{0.},0.,&p);}
  p.enabled=false;let mut tail=0.;for i in 0..(sr*0.1) as usize {let(l,rr)=r.process_stereo(0.,0.,&p);if i>(sr*0.006) as usize{tail+=l*l+rr*rr;}}
  if policy==ReverbTailPolicy::Kill {assert_eq!(tail,0.);assert!(r.is_dormant());}else{assert!(tail>1e-10);}
  if policy==ReverbTailPolicy::Kill {p.enabled=true;for _ in 0..(sr*0.2) as usize {assert_eq!(r.process_stereo(0.,0.,&p),(0.,0.));}}
 }}
}
#[test]
fn kill_fades_dry_gain_and_accepts_new_audio_without_old_history(){
 let mut r=SolarisCloudburstReverb::new(48000.);let mut p=isolated();p.enabled=false;p.bypass_tail=ReverbTailPolicy::Kill;
 let mut last=0.;for _ in 0..300 {let(l,_)=r.process_stereo(0.2,0.2,&p);assert!((l-last).abs()<0.003);last=l;}
 assert!((last-0.2).abs()<1e-6);p.enabled=true;let mut energy=0.;
 for i in 0..9600 {let(l,rr)=r.process_stereo(if i<100{0.2}else{0.},0.,&p);energy+=l*l+rr*rr;}
 assert!(energy>0.001);
}
#[test]
fn reset_invalidates_every_delay_and_restores_rng(){
 let mut old=SolarisCloudburstReverb::new(48000.);let p=SolarisReverbParams{wet_mix:1.,..Default::default()};
 for i in 0..96000 {old.process_stereo(if i%71==0{0.4}else{0.},0.,&p);}
 old.reset();let mut fresh=SolarisCloudburstReverb::new(48000.);
 for i in 0..48000 {let x=if i%101==0{0.2}else{0.};assert_eq!(old.process_stereo(x,0.,&p),fresh.process_stereo(x,0.,&p),"frame {i}");}
}
#[test]
fn wet_dry_law_is_affine_for_the_same_tank(){
 let mut a=SolarisCloudburstReverb::new(48000.);let mut b=SolarisCloudburstReverb::new(48000.);let mut p=isolated();let mut q=p;q.wet_mix=0.25;
 for i in 0..9600 {let x=if i%73==0{0.1}else{0.};let(w,_)=a.process_stereo(x,0.,&p);let(y,_)=b.process_stereo(x,0.,&q);assert!((y-(x*0.75+w*0.25)).abs()<1e-7);}
 p.wet_mix=0.;assert_eq!(a.process_stereo(0.25,-0.25,&p),(0.25,-0.25));
}
#[test]
fn ensemble_shimmer_sensitivity_and_freeze_are_finite(){
 for sr in [48000.,96000.] { let mut a=SolarisCloudburstReverb::new(sr);let mut b=SolarisCloudburstReverb::new(sr);
 let mut p=isolated();p.decay_sec=1.;let mut q=p;q.ensemble_mode=EnsembleMode::Forte;q.shimmer_amount=0.8;
 let mut difference=0.;for i in 0..sr as usize {let x=if i%97==0{0.1}else{0.};let(l,r)=a.process_stereo(x,x,&p);let(ll,rr)=b.process_stereo(x,x,&q);assert!([l,r,ll,rr].iter().all(|v|v.is_finite()&&v.abs()<8.));difference+=(l-ll).powi(2)+(r-rr).powi(2);}
 assert!(difference>0.001);q.freeze=true;for _ in 0..sr as usize{let(l,r)=b.process_stereo(1.,1.,&q);assert!(l.is_finite()&&r.is_finite()&&l.abs()<8.&&r.abs()<8.);}
 }
}
