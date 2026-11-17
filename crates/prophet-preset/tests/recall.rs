use prophet_control::music::{Kind, PatternBank};
use prophet_dsp::{
    fx::reverb::Algorithm,
    vector::{Route, Topology},
};
use prophet_preset::{Error, Preset};
#[test]
fn migrate_all_factory_presets_and_recall() {
    for i in 0..10 {
        let bytes = std::fs::read(format!(
            "{}/../../presets/{i:02}-factory.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let p = Preset::decode(&bytes).unwrap();
        assert_eq!(p.sound.algorithm, Algorithm::LegacyRC1);
        for rate in [44100, 48000, 96000] {
            let e = p.sound.prepare(rate, 480).unwrap();
            assert_eq!(e.sound_state().values, p.sound.values);
            assert_eq!(e.patch().topology, Topology::Classic2);
        }
        let new = p.encode().unwrap();
        assert_eq!(Preset::decode(&new).unwrap().encode().unwrap(), new);
    }
}
#[test]
fn checksum_and_vector_pattern_roundtrip() {
    let mut p = Preset::decode(include_bytes!("../../../presets/00-factory.json")).unwrap();
    p.sound.patch.topology = Topology::Vector4;
    p.sound.patch.routes[1] = Route::Sync;
    p.sound.patch.modulation[1].x = 0.75;
    p.sound.algorithm = Algorithm::DiffuseV1;
    p.sound.glide = false;
    let mut bank = Box::<PatternBank>::default();
    bank.0[63].steps[31].kind = Kind::Tie;
    p.bank = Some(bank);
    p.performance = Some(prophet_preset::Performance {
        tempo_milli: 123456,
        swing: 25,
        pattern: 63,
        mode: 4,
        octaves: 3,
        latch: true,
        seed: 1823,
    });
    let bytes = p.encode().unwrap();
    let q = Preset::decode(&bytes).unwrap();
    assert_eq!(q.bank.as_ref().unwrap().0[63].steps[31].kind, Kind::Tie);
    assert_eq!(p.performance, q.performance);
    let (_, arp) = q.performance.unwrap().prepare(44100).unwrap();
    assert_eq!(arp.seed, 1823);
    let e = q.sound.prepare(48000, 64).unwrap();
    assert_eq!(e.patch(), p.sound.patch);
    assert_eq!(e.reverb_state().algorithm, Algorithm::DiffuseV1);
    let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    json["payload"]["glide"] = true.into();
    assert!(matches!(
        Preset::decode(&serde_json::to_vec(&json).unwrap()),
        Err(Error::Checksum)
    ));
    assert!(Preset::decode(&bytes[..100]).is_err());
}
