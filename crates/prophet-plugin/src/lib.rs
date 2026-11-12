// SPDX-License-Identifier: GPL-3.0-only
use nih_plug::prelude::*;
use prophet_control::ProphetXiiPlugin;
use prophet_params::{AtomicParamBank, ParamId, NUM_PARAMS, PARAM_SPECS};
use std::sync::Arc;
#[derive(Params)]
struct SynthParams {
    #[id = "reverb_enabled"]
    reverb_enabled: BoolParam,
    #[id = "reverb_kill_tail"]
    reverb_kill_tail: BoolParam,
    #[id = "p0"]
    p0: FloatParam,
    #[id = "p1"]
    p1: FloatParam,
    #[id = "p2"]
    p2: FloatParam,
    #[id = "p3"]
    p3: FloatParam,
    #[id = "p4"]
    p4: FloatParam,
    #[id = "p5"]
    p5: FloatParam,
    #[id = "p6"]
    p6: FloatParam,
    #[id = "p7"]
    p7: FloatParam,
    #[id = "p8"]
    p8: FloatParam,
    #[id = "p9"]
    p9: FloatParam,
    #[id = "p10"]
    p10: FloatParam,
    #[id = "p11"]
    p11: FloatParam,
    #[id = "p12"]
    p12: FloatParam,
    #[id = "p13"]
    p13: FloatParam,
    #[id = "p14"]
    p14: FloatParam,
    #[id = "p15"]
    p15: FloatParam,
    #[id = "p16"]
    p16: FloatParam,
    #[id = "p17"]
    p17: FloatParam,
    #[id = "p18"]
    p18: FloatParam,
    #[id = "p19"]
    p19: FloatParam,
    #[id = "p20"]
    p20: FloatParam,
    #[id = "p21"]
    p21: FloatParam,
    #[id = "p22"]
    p22: FloatParam,
    #[id = "p23"]
    p23: FloatParam,
    #[id = "p24"]
    p24: FloatParam,
    #[id = "p25"]
    p25: FloatParam,
    #[id = "p26"]
    p26: FloatParam,
    #[id = "p27"]
    p27: FloatParam,
    #[id = "p28"]
    p28: FloatParam,
    #[id = "p29"]
    p29: FloatParam,
    #[id = "p30"]
    p30: FloatParam,
    #[id = "p31"]
    p31: FloatParam,
    #[id = "p32"]
    p32: FloatParam,
    #[id = "p33"]
    p33: FloatParam,
    #[id = "p34"]
    p34: FloatParam,
    #[id = "p35"]
    p35: FloatParam,
    #[id = "p36"]
    p36: FloatParam,
    #[id = "p37"]
    p37: FloatParam,
    #[id = "p38"]
    p38: FloatParam,
    #[id = "p39"]
    p39: FloatParam,
    #[id = "p40"]
    p40: FloatParam,
    #[id = "p41"]
    p41: FloatParam,
    #[id = "p42"]
    p42: FloatParam,
    #[id = "p43"]
    p43: FloatParam,
    #[id = "p44"]
    p44: FloatParam,
    #[id = "p45"]
    p45: FloatParam,
    #[id = "p46"]
    p46: FloatParam,
    #[id = "p47"]
    p47: FloatParam,
    #[id = "p48"]
    p48: FloatParam,
    #[id = "p49"]
    p49: FloatParam,
    #[id = "p50"]
    p50: FloatParam,
    #[id = "p51"]
    p51: FloatParam,
    #[id = "p52"]
    p52: FloatParam,
    #[id = "p53"]
    p53: FloatParam,
    #[id = "p54"]
    p54: FloatParam,
    #[id = "p55"]
    p55: FloatParam,
    #[id = "p56"]
    p56: FloatParam,
    #[id = "p57"]
    p57: FloatParam,
    #[id = "p58"]
    p58: FloatParam,
    #[id = "p59"]
    p59: FloatParam,
    #[id = "p60"]
    p60: FloatParam,
    #[id = "p61"]
    p61: FloatParam,
    #[id = "p62"]
    p62: FloatParam,
    #[id = "p63"]
    p63: FloatParam,
    #[id = "p64"]
    p64: FloatParam,
    #[id = "p65"]
    p65: FloatParam,
    #[id = "p66"]
    p66: FloatParam,
    #[id = "p67"]
    p67: FloatParam,
    #[id = "p68"]
    p68: FloatParam,
    #[id = "p69"]
    p69: FloatParam,
    #[id = "p70"]
    p70: FloatParam,
    #[id = "p71"]
    p71: FloatParam,
    #[id = "p72"]
    p72: FloatParam,
    #[id = "p73"]
    p73: FloatParam,
    #[id = "p74"]
    p74: FloatParam,
    #[id = "p75"]
    p75: FloatParam,
    #[id = "p76"]
    p76: FloatParam,
    #[id = "p77"]
    p77: FloatParam,
    #[id = "p78"]
    p78: FloatParam,
    #[id = "p79"]
    p79: FloatParam,
    #[id = "p80"]
    p80: FloatParam,
    #[id = "p81"]
    p81: FloatParam,
    #[id = "p82"]
    p82: FloatParam,
}
impl Default for SynthParams {
    fn default() -> Self {
        let bank = AtomicParamBank::new();
        Self {
            reverb_enabled: BoolParam::new("Reverb Enabled", true),
            reverb_kill_tail: BoolParam::new("Reverb Kill Tail (off = Spill)", false),
            p0: FloatParam::new(
                PARAM_SPECS[0].name,
                bank.get(ParamId::from_index(0).unwrap()),
                FloatRange::Linear {
                    min: -36.0,
                    max: 36.0,
                },
            )
            .with_step_size(1.0),
            p1: FloatParam::new(
                PARAM_SPECS[1].name,
                bank.get(ParamId::from_index(1).unwrap()),
                FloatRange::Linear {
                    min: -100.0,
                    max: 100.0,
                },
            ),
            p2: FloatParam::new(
                PARAM_SPECS[2].name,
                bank.get(ParamId::from_index(2).unwrap()),
                FloatRange::Linear {
                    min: 0.01,
                    max: 0.99,
                },
            ),
            p3: FloatParam::new(
                PARAM_SPECS[3].name,
                bank.get(ParamId::from_index(3).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p4: FloatParam::new(
                PARAM_SPECS[4].name,
                bank.get(ParamId::from_index(4).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p5: FloatParam::new(
                PARAM_SPECS[5].name,
                bank.get(ParamId::from_index(5).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p6: FloatParam::new(
                PARAM_SPECS[6].name,
                bank.get(ParamId::from_index(6).unwrap()),
                FloatRange::Linear {
                    min: -36.0,
                    max: 36.0,
                },
            )
            .with_step_size(1.0),
            p7: FloatParam::new(
                PARAM_SPECS[7].name,
                bank.get(ParamId::from_index(7).unwrap()),
                FloatRange::Linear {
                    min: -100.0,
                    max: 100.0,
                },
            ),
            p8: FloatParam::new(
                PARAM_SPECS[8].name,
                bank.get(ParamId::from_index(8).unwrap()),
                FloatRange::Linear {
                    min: 0.01,
                    max: 0.99,
                },
            ),
            p9: FloatParam::new(
                PARAM_SPECS[9].name,
                bank.get(ParamId::from_index(9).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p10: FloatParam::new(
                PARAM_SPECS[10].name,
                bank.get(ParamId::from_index(10).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p11: FloatParam::new(
                PARAM_SPECS[11].name,
                bank.get(ParamId::from_index(11).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p12: FloatParam::new(
                PARAM_SPECS[12].name,
                bank.get(ParamId::from_index(12).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p13: FloatParam::new(
                PARAM_SPECS[13].name,
                bank.get(ParamId::from_index(13).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p14: FloatParam::new(
                PARAM_SPECS[14].name,
                bank.get(ParamId::from_index(14).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p15: FloatParam::new(
                PARAM_SPECS[15].name,
                bank.get(ParamId::from_index(15).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p16: FloatParam::new(
                PARAM_SPECS[16].name,
                bank.get(ParamId::from_index(16).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p17: FloatParam::new(
                PARAM_SPECS[17].name,
                bank.get(ParamId::from_index(17).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p18: FloatParam::new(
                PARAM_SPECS[18].name,
                bank.get(ParamId::from_index(18).unwrap()),
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            ),
            p19: FloatParam::new(
                PARAM_SPECS[19].name,
                bank.get(ParamId::from_index(19).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p20: FloatParam::new(
                PARAM_SPECS[20].name,
                bank.get(ParamId::from_index(20).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p21: FloatParam::new(
                PARAM_SPECS[21].name,
                bank.get(ParamId::from_index(21).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p22: FloatParam::new(
                PARAM_SPECS[22].name,
                bank.get(ParamId::from_index(22).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p23: FloatParam::new(
                PARAM_SPECS[23].name,
                bank.get(ParamId::from_index(23).unwrap()),
                FloatRange::Linear {
                    min: 20.0,
                    max: 18000.0,
                },
            ),
            p24: FloatParam::new(
                PARAM_SPECS[24].name,
                bank.get(ParamId::from_index(24).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p25: FloatParam::new(
                PARAM_SPECS[25].name,
                bank.get(ParamId::from_index(25).unwrap()),
                FloatRange::Linear {
                    min: 0.25,
                    max: 4.0,
                },
            ),
            p26: FloatParam::new(
                PARAM_SPECS[26].name,
                bank.get(ParamId::from_index(26).unwrap()),
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            ),
            p27: FloatParam::new(
                PARAM_SPECS[27].name,
                bank.get(ParamId::from_index(27).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p28: FloatParam::new(
                PARAM_SPECS[28].name,
                bank.get(ParamId::from_index(28).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p29: FloatParam::new(
                PARAM_SPECS[29].name,
                bank.get(ParamId::from_index(29).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p30: FloatParam::new(
                PARAM_SPECS[30].name,
                bank.get(ParamId::from_index(30).unwrap()),
                FloatRange::Linear {
                    min: 0.001,
                    max: 25.0,
                },
            ),
            p31: FloatParam::new(
                PARAM_SPECS[31].name,
                bank.get(ParamId::from_index(31).unwrap()),
                FloatRange::Linear {
                    min: 0.001,
                    max: 25.0,
                },
            ),
            p32: FloatParam::new(
                PARAM_SPECS[32].name,
                bank.get(ParamId::from_index(32).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p33: FloatParam::new(
                PARAM_SPECS[33].name,
                bank.get(ParamId::from_index(33).unwrap()),
                FloatRange::Linear {
                    min: 0.001,
                    max: 25.0,
                },
            ),
            p34: FloatParam::new(
                PARAM_SPECS[34].name,
                bank.get(ParamId::from_index(34).unwrap()),
                FloatRange::Linear {
                    min: 0.001,
                    max: 25.0,
                },
            ),
            p35: FloatParam::new(
                PARAM_SPECS[35].name,
                bank.get(ParamId::from_index(35).unwrap()),
                FloatRange::Linear {
                    min: 0.001,
                    max: 25.0,
                },
            ),
            p36: FloatParam::new(
                PARAM_SPECS[36].name,
                bank.get(ParamId::from_index(36).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p37: FloatParam::new(
                PARAM_SPECS[37].name,
                bank.get(ParamId::from_index(37).unwrap()),
                FloatRange::Linear {
                    min: 0.001,
                    max: 25.0,
                },
            ),
            p38: FloatParam::new(
                PARAM_SPECS[38].name,
                bank.get(ParamId::from_index(38).unwrap()),
                FloatRange::Linear {
                    min: 0.04,
                    max: 30.0,
                },
            ),
            p39: FloatParam::new(
                PARAM_SPECS[39].name,
                bank.get(ParamId::from_index(39).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p40: FloatParam::new(
                PARAM_SPECS[40].name,
                bank.get(ParamId::from_index(40).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p41: FloatParam::new(
                PARAM_SPECS[41].name,
                bank.get(ParamId::from_index(41).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.5 },
            ),
            p42: FloatParam::new(
                PARAM_SPECS[42].name,
                bank.get(ParamId::from_index(42).unwrap()),
                FloatRange::Linear { min: 0.0, max: 5.0 },
            ),
            p43: FloatParam::new(
                PARAM_SPECS[43].name,
                bank.get(ParamId::from_index(43).unwrap()),
                FloatRange::Linear {
                    min: 1.0,
                    max: 12.0,
                },
            )
            .with_step_size(1.0),
            p44: FloatParam::new(
                PARAM_SPECS[44].name,
                bank.get(ParamId::from_index(44).unwrap()),
                FloatRange::Linear {
                    min: 0.0,
                    max: 60.0,
                },
            ),
            p45: FloatParam::new(
                PARAM_SPECS[45].name,
                bank.get(ParamId::from_index(45).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p46: FloatParam::new(
                PARAM_SPECS[46].name,
                bank.get(ParamId::from_index(46).unwrap()),
                FloatRange::Linear { min: 0.0, max: 3.0 },
            )
            .with_step_size(1.0),
            p47: FloatParam::new(
                PARAM_SPECS[47].name,
                bank.get(ParamId::from_index(47).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p48: FloatParam::new(
                PARAM_SPECS[48].name,
                bank.get(ParamId::from_index(48).unwrap()),
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.005,
                },
            ),
            p49: FloatParam::new(
                PARAM_SPECS[49].name,
                bank.get(ParamId::from_index(49).unwrap()),
                FloatRange::Linear {
                    min: 0.0,
                    max: 500.0,
                },
            ),
            p50: FloatParam::new(
                PARAM_SPECS[50].name,
                bank.get(ParamId::from_index(50).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p51: FloatParam::new(
                PARAM_SPECS[51].name,
                bank.get(ParamId::from_index(51).unwrap()),
                FloatRange::Linear { min: 0.0, max: 2.0 },
            )
            .with_step_size(1.0),
            p52: FloatParam::new(
                PARAM_SPECS[52].name,
                bank.get(ParamId::from_index(52).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p53: FloatParam::new(
                PARAM_SPECS[53].name,
                bank.get(ParamId::from_index(53).unwrap()),
                FloatRange::Linear {
                    min: 0.2,
                    max: 60.0,
                },
            ),
            p54: FloatParam::new(
                PARAM_SPECS[54].name,
                bank.get(ParamId::from_index(54).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p55: FloatParam::new(
                PARAM_SPECS[55].name,
                bank.get(ParamId::from_index(55).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p56: FloatParam::new(
                PARAM_SPECS[56].name,
                bank.get(ParamId::from_index(56).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p57: FloatParam::new(
                PARAM_SPECS[57].name,
                bank.get(ParamId::from_index(57).unwrap()),
                FloatRange::Linear {
                    min: 800.0,
                    max: 18000.0,
                },
            ),
            p58: FloatParam::new(
                PARAM_SPECS[58].name,
                bank.get(ParamId::from_index(58).unwrap()),
                FloatRange::Linear {
                    min: 20.0,
                    max: 1000.0,
                },
            ),
            p59: FloatParam::new(
                PARAM_SPECS[59].name,
                bank.get(ParamId::from_index(59).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p60: FloatParam::new(
                PARAM_SPECS[60].name,
                bank.get(ParamId::from_index(60).unwrap()),
                FloatRange::Linear {
                    min: 0.05,
                    max: 5.0,
                },
            ),
            p61: FloatParam::new(
                PARAM_SPECS[61].name,
                bank.get(ParamId::from_index(61).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p62: FloatParam::new(
                PARAM_SPECS[62].name,
                bank.get(ParamId::from_index(62).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p63: FloatParam::new(
                PARAM_SPECS[63].name,
                bank.get(ParamId::from_index(63).unwrap()),
                FloatRange::Linear { min: 1.0, max: 4.0 },
            )
            .with_step_size(1.0),
            p64: FloatParam::new(
                PARAM_SPECS[64].name,
                bank.get(ParamId::from_index(64).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p65: FloatParam::new(
                PARAM_SPECS[65].name,
                bank.get(ParamId::from_index(65).unwrap()),
                FloatRange::Linear {
                    min: -7.0,
                    max: 7.0,
                },
            ),
            p66: FloatParam::new(
                PARAM_SPECS[66].name,
                bank.get(ParamId::from_index(66).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p67: FloatParam::new(
                PARAM_SPECS[67].name,
                bank.get(ParamId::from_index(67).unwrap()),
                FloatRange::Linear { min: 0.0, max: 2.0 },
            )
            .with_step_size(1.0),
            p68: FloatParam::new(
                PARAM_SPECS[68].name,
                bank.get(ParamId::from_index(68).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p69: FloatParam::new(
                PARAM_SPECS[69].name,
                bank.get(ParamId::from_index(69).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p70: FloatParam::new(
                PARAM_SPECS[70].name,
                bank.get(ParamId::from_index(70).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p71: FloatParam::new(
                PARAM_SPECS[71].name,
                bank.get(ParamId::from_index(71).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p72: FloatParam::new(
                PARAM_SPECS[72].name,
                bank.get(ParamId::from_index(72).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p73: FloatParam::new(
                PARAM_SPECS[73].name,
                bank.get(ParamId::from_index(73).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p74: FloatParam::new(
                PARAM_SPECS[74].name,
                bank.get(ParamId::from_index(74).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p75: FloatParam::new(
                PARAM_SPECS[75].name,
                bank.get(ParamId::from_index(75).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p76: FloatParam::new(
                PARAM_SPECS[76].name,
                bank.get(ParamId::from_index(76).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            p77: FloatParam::new(
                PARAM_SPECS[77].name,
                bank.get(ParamId::from_index(77).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p78: FloatParam::new(
                PARAM_SPECS[78].name,
                bank.get(ParamId::from_index(78).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p79: FloatParam::new(
                PARAM_SPECS[79].name,
                bank.get(ParamId::from_index(79).unwrap()),
                FloatRange::Linear {
                    min: -12.0,
                    max: 12.0,
                },
            ),
            p80: FloatParam::new(
                PARAM_SPECS[80].name,
                bank.get(ParamId::from_index(80).unwrap()),
                FloatRange::Linear {
                    min: 20.0,
                    max: 300.0,
                },
            ),
            p81: FloatParam::new(
                PARAM_SPECS[81].name,
                bank.get(ParamId::from_index(81).unwrap()),
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_step_size(1.0),
            p82: FloatParam::new(
                PARAM_SPECS[82].name,
                bank.get(ParamId::from_index(82).unwrap()),
                FloatRange::Linear { min: 0.0, max: 4.0 },
            ),
        }
    }
}
pub struct Prophet {
    params: Arc<SynthParams>,
    engine: ProphetXiiPlugin,
    last_host: [f32; NUM_PARAMS],
}
impl Default for Prophet {
    fn default() -> Self {
        Self {
            params: Arc::new(SynthParams::default()),
            engine: ProphetXiiPlugin::new(48000.0),
            last_host: [f32::NAN; NUM_PARAMS],
        }
    }
}
impl Prophet {
    fn pull(&mut self) {
        use prophet_dsp::fx::solaris_reverb::ReverbTailPolicy;
        self.engine.ipc.params.set_reverb_bypass(
            self.params.reverb_enabled.value(),
            if self.params.reverb_kill_tail.value() {
                ReverbTailPolicy::Kill
            } else {
                ReverbTailPolicy::Spill
            },
        );
        let value = self.params.p0.value();
        if value != self.last_host[0] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(0).unwrap(), value);
            self.last_host[0] = value;
        }
        let value = self.params.p1.value();
        if value != self.last_host[1] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(1).unwrap(), value);
            self.last_host[1] = value;
        }
        let value = self.params.p2.value();
        if value != self.last_host[2] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(2).unwrap(), value);
            self.last_host[2] = value;
        }
        let value = self.params.p3.value();
        if value != self.last_host[3] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(3).unwrap(), value);
            self.last_host[3] = value;
        }
        let value = self.params.p4.value();
        if value != self.last_host[4] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(4).unwrap(), value);
            self.last_host[4] = value;
        }
        let value = self.params.p5.value();
        if value != self.last_host[5] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(5).unwrap(), value);
            self.last_host[5] = value;
        }
        let value = self.params.p6.value();
        if value != self.last_host[6] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(6).unwrap(), value);
            self.last_host[6] = value;
        }
        let value = self.params.p7.value();
        if value != self.last_host[7] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(7).unwrap(), value);
            self.last_host[7] = value;
        }
        let value = self.params.p8.value();
        if value != self.last_host[8] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(8).unwrap(), value);
            self.last_host[8] = value;
        }
        let value = self.params.p9.value();
        if value != self.last_host[9] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(9).unwrap(), value);
            self.last_host[9] = value;
        }
        let value = self.params.p10.value();
        if value != self.last_host[10] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(10).unwrap(), value);
            self.last_host[10] = value;
        }
        let value = self.params.p11.value();
        if value != self.last_host[11] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(11).unwrap(), value);
            self.last_host[11] = value;
        }
        let value = self.params.p12.value();
        if value != self.last_host[12] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(12).unwrap(), value);
            self.last_host[12] = value;
        }
        let value = self.params.p13.value();
        if value != self.last_host[13] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(13).unwrap(), value);
            self.last_host[13] = value;
        }
        let value = self.params.p14.value();
        if value != self.last_host[14] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(14).unwrap(), value);
            self.last_host[14] = value;
        }
        let value = self.params.p15.value();
        if value != self.last_host[15] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(15).unwrap(), value);
            self.last_host[15] = value;
        }
        let value = self.params.p16.value();
        if value != self.last_host[16] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(16).unwrap(), value);
            self.last_host[16] = value;
        }
        let value = self.params.p17.value();
        if value != self.last_host[17] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(17).unwrap(), value);
            self.last_host[17] = value;
        }
        let value = self.params.p18.value();
        if value != self.last_host[18] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(18).unwrap(), value);
            self.last_host[18] = value;
        }
        let value = self.params.p19.value();
        if value != self.last_host[19] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(19).unwrap(), value);
            self.last_host[19] = value;
        }
        let value = self.params.p20.value();
        if value != self.last_host[20] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(20).unwrap(), value);
            self.last_host[20] = value;
        }
        let value = self.params.p21.value();
        if value != self.last_host[21] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(21).unwrap(), value);
            self.last_host[21] = value;
        }
        let value = self.params.p22.value();
        if value != self.last_host[22] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(22).unwrap(), value);
            self.last_host[22] = value;
        }
        let value = self.params.p23.value();
        if value != self.last_host[23] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(23).unwrap(), value);
            self.last_host[23] = value;
        }
        let value = self.params.p24.value();
        if value != self.last_host[24] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(24).unwrap(), value);
            self.last_host[24] = value;
        }
        let value = self.params.p25.value();
        if value != self.last_host[25] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(25).unwrap(), value);
            self.last_host[25] = value;
        }
        let value = self.params.p26.value();
        if value != self.last_host[26] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(26).unwrap(), value);
            self.last_host[26] = value;
        }
        let value = self.params.p27.value();
        if value != self.last_host[27] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(27).unwrap(), value);
            self.last_host[27] = value;
        }
        let value = self.params.p28.value();
        if value != self.last_host[28] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(28).unwrap(), value);
            self.last_host[28] = value;
        }
        let value = self.params.p29.value();
        if value != self.last_host[29] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(29).unwrap(), value);
            self.last_host[29] = value;
        }
        let value = self.params.p30.value();
        if value != self.last_host[30] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(30).unwrap(), value);
            self.last_host[30] = value;
        }
        let value = self.params.p31.value();
        if value != self.last_host[31] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(31).unwrap(), value);
            self.last_host[31] = value;
        }
        let value = self.params.p32.value();
        if value != self.last_host[32] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(32).unwrap(), value);
            self.last_host[32] = value;
        }
        let value = self.params.p33.value();
        if value != self.last_host[33] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(33).unwrap(), value);
            self.last_host[33] = value;
        }
        let value = self.params.p34.value();
        if value != self.last_host[34] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(34).unwrap(), value);
            self.last_host[34] = value;
        }
        let value = self.params.p35.value();
        if value != self.last_host[35] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(35).unwrap(), value);
            self.last_host[35] = value;
        }
        let value = self.params.p36.value();
        if value != self.last_host[36] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(36).unwrap(), value);
            self.last_host[36] = value;
        }
        let value = self.params.p37.value();
        if value != self.last_host[37] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(37).unwrap(), value);
            self.last_host[37] = value;
        }
        let value = self.params.p38.value();
        if value != self.last_host[38] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(38).unwrap(), value);
            self.last_host[38] = value;
        }
        let value = self.params.p39.value();
        if value != self.last_host[39] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(39).unwrap(), value);
            self.last_host[39] = value;
        }
        let value = self.params.p40.value();
        if value != self.last_host[40] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(40).unwrap(), value);
            self.last_host[40] = value;
        }
        let value = self.params.p41.value();
        if value != self.last_host[41] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(41).unwrap(), value);
            self.last_host[41] = value;
        }
        let value = self.params.p42.value();
        if value != self.last_host[42] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(42).unwrap(), value);
            self.last_host[42] = value;
        }
        let value = self.params.p43.value();
        if value != self.last_host[43] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(43).unwrap(), value);
            self.last_host[43] = value;
        }
        let value = self.params.p44.value();
        if value != self.last_host[44] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(44).unwrap(), value);
            self.last_host[44] = value;
        }
        let value = self.params.p45.value();
        if value != self.last_host[45] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(45).unwrap(), value);
            self.last_host[45] = value;
        }
        let value = self.params.p46.value();
        if value != self.last_host[46] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(46).unwrap(), value);
            self.last_host[46] = value;
        }
        let value = self.params.p47.value();
        if value != self.last_host[47] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(47).unwrap(), value);
            self.last_host[47] = value;
        }
        let value = self.params.p48.value();
        if value != self.last_host[48] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(48).unwrap(), value);
            self.last_host[48] = value;
        }
        let value = self.params.p49.value();
        if value != self.last_host[49] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(49).unwrap(), value);
            self.last_host[49] = value;
        }
        let value = self.params.p50.value();
        if value != self.last_host[50] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(50).unwrap(), value);
            self.last_host[50] = value;
        }
        let value = self.params.p51.value();
        if value != self.last_host[51] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(51).unwrap(), value);
            self.last_host[51] = value;
        }
        let value = self.params.p52.value();
        if value != self.last_host[52] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(52).unwrap(), value);
            self.last_host[52] = value;
        }
        let value = self.params.p53.value();
        if value != self.last_host[53] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(53).unwrap(), value);
            self.last_host[53] = value;
        }
        let value = self.params.p54.value();
        if value != self.last_host[54] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(54).unwrap(), value);
            self.last_host[54] = value;
        }
        let value = self.params.p55.value();
        if value != self.last_host[55] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(55).unwrap(), value);
            self.last_host[55] = value;
        }
        let value = self.params.p56.value();
        if value != self.last_host[56] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(56).unwrap(), value);
            self.last_host[56] = value;
        }
        let value = self.params.p57.value();
        if value != self.last_host[57] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(57).unwrap(), value);
            self.last_host[57] = value;
        }
        let value = self.params.p58.value();
        if value != self.last_host[58] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(58).unwrap(), value);
            self.last_host[58] = value;
        }
        let value = self.params.p59.value();
        if value != self.last_host[59] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(59).unwrap(), value);
            self.last_host[59] = value;
        }
        let value = self.params.p60.value();
        if value != self.last_host[60] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(60).unwrap(), value);
            self.last_host[60] = value;
        }
        let value = self.params.p61.value();
        if value != self.last_host[61] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(61).unwrap(), value);
            self.last_host[61] = value;
        }
        let value = self.params.p62.value();
        if value != self.last_host[62] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(62).unwrap(), value);
            self.last_host[62] = value;
        }
        let value = self.params.p63.value();
        if value != self.last_host[63] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(63).unwrap(), value);
            self.last_host[63] = value;
        }
        let value = self.params.p64.value();
        if value != self.last_host[64] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(64).unwrap(), value);
            self.last_host[64] = value;
        }
        let value = self.params.p65.value();
        if value != self.last_host[65] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(65).unwrap(), value);
            self.last_host[65] = value;
        }
        let value = self.params.p66.value();
        if value != self.last_host[66] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(66).unwrap(), value);
            self.last_host[66] = value;
        }
        let value = self.params.p67.value();
        if value != self.last_host[67] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(67).unwrap(), value);
            self.last_host[67] = value;
        }
        let value = self.params.p68.value();
        if value != self.last_host[68] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(68).unwrap(), value);
            self.last_host[68] = value;
        }
        let value = self.params.p69.value();
        if value != self.last_host[69] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(69).unwrap(), value);
            self.last_host[69] = value;
        }
        let value = self.params.p70.value();
        if value != self.last_host[70] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(70).unwrap(), value);
            self.last_host[70] = value;
        }
        let value = self.params.p71.value();
        if value != self.last_host[71] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(71).unwrap(), value);
            self.last_host[71] = value;
        }
        let value = self.params.p72.value();
        if value != self.last_host[72] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(72).unwrap(), value);
            self.last_host[72] = value;
        }
        let value = self.params.p73.value();
        if value != self.last_host[73] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(73).unwrap(), value);
            self.last_host[73] = value;
        }
        let value = self.params.p74.value();
        if value != self.last_host[74] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(74).unwrap(), value);
            self.last_host[74] = value;
        }
        let value = self.params.p75.value();
        if value != self.last_host[75] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(75).unwrap(), value);
            self.last_host[75] = value;
        }
        let value = self.params.p76.value();
        if value != self.last_host[76] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(76).unwrap(), value);
            self.last_host[76] = value;
        }
        let value = self.params.p77.value();
        if value != self.last_host[77] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(77).unwrap(), value);
            self.last_host[77] = value;
        }
        let value = self.params.p78.value();
        if value != self.last_host[78] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(78).unwrap(), value);
            self.last_host[78] = value;
        }
        let value = self.params.p79.value();
        if value != self.last_host[79] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(79).unwrap(), value);
            self.last_host[79] = value;
        }
        let value = self.params.p80.value();
        if value != self.last_host[80] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(80).unwrap(), value);
            self.last_host[80] = value;
        }
        let value = self.params.p81.value();
        if value != self.last_host[81] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(81).unwrap(), value);
            self.last_host[81] = value;
        }
        let value = self.params.p82.value();
        if value != self.last_host[82] {
            self.engine
                .ipc
                .params
                .set(ParamId::from_index(82).unwrap(), value);
            self.last_host[82] = value;
        }
        self.engine.sync_parameters();
    }
}
impl Plugin for Prophet {
    const NAME: &'static str = "PROPHET-XII";
    const VENDOR: &'static str = "PROPHET-XII Contributors";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    type SysExMessage = ();
    type BackgroundTask = ();
    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }
    fn initialize(
        &mut self,
        _: &AudioIOLayout,
        cfg: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.engine = ProphetXiiPlugin::new(cfg.sample_rate);
        self.last_host = [f32::NAN; NUM_PARAMS];
        self.pull();
        context.set_latency_samples(
            self.engine.kernel.allocator.voices[0]
                .oversampler
                .factor
                .latency_samples(),
        );
        true
    }
    fn reset(&mut self) {
        self.engine.panic();
        self.engine.kernel.reset();
    }
    fn process(
        &mut self,
        buffer: &mut Buffer,
        _: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.pull();
        context.set_latency_samples(
            self.engine.kernel.allocator.voices[0]
                .oversampler
                .factor
                .latency_samples(),
        );
        let mut next = context.next_event();
        for (i, channels) in buffer.iter_samples().enumerate() {
            while let Some(event) = next {
                if event.timing() > i as u32 {
                    break;
                }
                match event {
                    NoteEvent::NoteOn {
                        channel,
                        note,
                        velocity,
                        ..
                    } => self.engine.handle_midi_message(
                        0x90 | channel,
                        note,
                        (velocity * 127.0).round() as u8,
                    ),
                    NoteEvent::NoteOff { channel, note, .. } => {
                        self.engine.handle_midi_message(0x80 | channel, note, 0)
                    }
                    NoteEvent::PolyPressure { note, pressure, .. } => {
                        for v in &mut self.engine.kernel.allocator.voices {
                            if v.midi_note == note {
                                v.mpe_pressure = pressure;
                            }
                        }
                    }
                    NoteEvent::MidiChannelPressure {
                        channel, pressure, ..
                    } => self.engine.handle_midi_message(
                        0xd0 | channel,
                        (pressure * 127.0).round() as u8,
                        0,
                    ),
                    NoteEvent::MidiPitchBend { channel, value, .. } => {
                        let v = (value * 16383.0).round() as u16;
                        self.engine.handle_midi_message(
                            0xe0 | channel,
                            (v & 127) as u8,
                            (v >> 7) as u8,
                        );
                    }
                    NoteEvent::MidiCC {
                        channel, cc, value, ..
                    } => self.engine.handle_midi_message(
                        0xb0 | channel,
                        cc,
                        (value * 127.0).round() as u8,
                    ),
                    _ => {}
                }
                self.engine.sync_parameters();
                next = context.next_event();
            }
            let mut l = [0.0];
            let mut r = [0.0];
            self.engine.kernel.process_block(&mut l, &mut r, 1);
            for (c, sample) in channels.into_iter().enumerate() {
                *sample = if c == 0 { l[0] } else { r[0] };
            }
        }
        ProcessStatus::KeepAlive
    }
}
impl ClapPlugin for Prophet {
    const CLAP_ID: &'static str = "org.prophet-xii.synth";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Independent 12-voice virtual analog instrument");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}
impl Vst3Plugin for Prophet {
    const VST3_CLASS_ID: [u8; 16] = *b"ProphetXiiSynth1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}
nih_export_clap!(Prophet);
nih_export_vst3!(Prophet);
