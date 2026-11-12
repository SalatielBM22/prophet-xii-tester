//! Schroeder energy-decay measurement, outside any callback. Issues #1/#4.
use prophet_dsp::fx::solaris_reverb::*;
fn main(){
 println!("sample_rate,profile,requested_seconds,t20_rt60_seconds,fit_r_squared,peak,stereo_correlation");
 for sr in [48000.,96000.] {for decay in [0.5,1.5,4.8] {for neutral in [true,false]{
 let mut p=SolarisReverbParams{wet_mix:1.,decay_sec:decay,pre_delay_ms:0.,..Default::default()};
 if neutral{p.ensemble_mode=EnsembleMode::Off;p.shimmer_amount=0.;p.vowel_mix=0.;p.mod_depth=0.;p.high_cut_hz=18000.;p.low_cut_hz=20.;}
 let mut r=SolarisCloudburstReverb::new(sr);let frames=((decay*4.+1.)*sr) as usize;
 let mut energy=Vec::with_capacity(frames);let(mut peak,mut lr,mut ll,mut rr)=(0f64,0f64,0f64,0f64);
 for i in 0..frames{let(l,right)=r.process_stereo(if i==0{0.5}else{0.},0.,&p);assert!(l.is_finite()&&right.is_finite());let(l,right)=(l as f64,right as f64);peak=peak.max(l.abs()).max(right.abs());lr+=l*right;ll+=l*l;rr+=right*right;energy.push(l*l+right*right);}
 let mut sum=0.;for e in energy.iter_mut().rev(){sum+=*e;*e=sum;}
 let norm=energy[0];let(mut count,mut x,mut y,mut xx,mut yy,mut xy)=(0f64,0f64,0f64,0f64,0f64,0f64);
 for(i,e)in energy.iter().enumerate(){let db=10.*(e/norm).max(1e-30).log10();if (-25. ..=-5.).contains(&db){let t=i as f64/sr as f64;count+=1.;x+=t;y+=db;xx+=t*t;yy+=db*db;xy+=t*db;}}
 assert!(count>10.);let slope=(count*xy-x*y)/(count*xx-x*x);let r2=(count*xy-x*y).powi(2)/((count*xx-x*x)*(count*yy-y*y));
 println!("{sr},{},{decay},{:.6},{r2:.6},{peak:.6},{:.6}",if neutral{"neutral"}else{"artistic-default"},-60./slope,lr/(ll*rr).sqrt());
 }}}
}
