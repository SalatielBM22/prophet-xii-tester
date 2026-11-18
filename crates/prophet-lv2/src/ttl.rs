// SPDX-License-Identifier: GPL-3.0-only
//! Deterministic Turtle (`.ttl`) manifest and plugin ontology generator for `PROPHET-XII.lv2`.

use crate::lv2_ffi::{PLUGIN_URI_STR, UI_URI_STR};
use prophet_params::{AtomicParamBank, ParamId, NUM_PARAMS, PARAM_SPECS};
use std::fmt::Write as _;

pub const PORT_OUT_L: u32 = 0;
pub const PORT_OUT_R: u32 = 1;
pub const PORT_CONTROL_IN: u32 = 2;
pub const PORT_NOTIFY_OUT: u32 = 3;
pub const PORT_REVERB_ENABLED: u32 = 4;
pub const PORT_REVERB_KILL_TAIL: u32 = 5;
pub const PORT_PARAMS_START: u32 = 6;
pub const PORT_GLIDE_ENABLED: u32 = PORT_PARAMS_START + NUM_PARAMS as u32; // 89
pub const PORT_LATENCY: u32 = PORT_GLIDE_ENABLED + 1; // 90
pub const TOTAL_PORTS: usize = (PORT_LATENCY as usize) + 1; // 91

#[inline]
pub fn param_id_to_port(id: u16) -> Option<u32> {
    let idx = usize::from(id);
    if idx < NUM_PARAMS {
        Some(PORT_PARAMS_START + idx as u32)
    } else if id == prophet_control::port::GLIDE_ENABLED {
        Some(PORT_GLIDE_ENABLED)
    } else {
        None
    }
}

fn fmt_ttl_num(val: f32, force_int: bool) -> String {
    if force_int {
        return format!("{}", val.round() as i64);
    }
    if (val - val.round()).abs() <= 1e-6 && val.abs() < 1_000_000.0 {
        return format!("{:.1}", val.round());
    }
    let s = format!("{val:.6}");
    let trimmed = s.trim_end_matches('0');
    if let Some(prefix) = trimmed.strip_suffix('.') {
        format!("{prefix}.0")
    } else {
        trimmed.to_owned()
    }
}

fn unit_for_param(id: ParamId) -> Option<&'static str> {
    match id {
        ParamId::FilterCutoff
        | ParamId::LfoFreq
        | ParamId::ReverbHighCut
        | ParamId::ReverbLowCut
        | ParamId::ReverbModSpeed => Some("units:hz"),
        ParamId::FiltAttack
        | ParamId::FiltDecay
        | ParamId::FiltRelease
        | ParamId::AmpAttack
        | ParamId::AmpDecay
        | ParamId::AmpRelease
        | ParamId::GlideTime
        | ParamId::ReverbDecay => Some("units:s"),
        ParamId::ReverbPreDelay => Some("units:ms"),
        ParamId::OscASemi
        | ParamId::OscBSemi
        | ParamId::PitchBend
        | ParamId::ShimmerSemitones => Some("units:semitone12TET"),
        ParamId::OscAFine | ParamId::OscBFine | ParamId::UnisonDetune => Some("units:cent"),
        ParamId::ReverbTempoBpm => Some("units:bpm"),
        ParamId::ReverbPreDelayBeats => Some("units:beat"),
        _ => None,
    }
}

fn scale_points_for_param(id: ParamId) -> &'static [(&'static str, i32)] {
    match id {
        ParamId::ChorusMode => &[("Off", 0), ("I", 1), ("II", 2), ("I+II", 3)],
        ParamId::ReverbEnsembleMode => &[("Off", 0), ("mp", 1), ("f", 2)],
        ParamId::OversamplingMode => &[("1x", 1), ("2x", 2), ("4x", 4)],
        ParamId::NotePriority => &[("Last", 0), ("Low", 1), ("High", 2)],
        ParamId::GlideMode => &[("Constant Rate", 0), ("Constant Time", 1)],
        _ => &[],
    }
}

pub fn generate_manifest_ttl(binary_filename: &str) -> String {
    format!(
        r#"@prefix lv2:  <http://lv2plug.in/ns/lv2core#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ui:   <http://lv2plug.in/ns/extensions/ui#> .

<{PLUGIN_URI_STR}>
	a lv2:Plugin ;
	lv2:binary <{binary_filename}> ;
	rdfs:seeAlso <prophet-xii.ttl> .

<{UI_URI_STR}>
	a ui:X11UI ;
	ui:binary <{binary_filename}> ;
	rdfs:seeAlso <prophet-xii.ttl> .
"#
    )
}

pub fn generate_plugin_ttl(binary_filename: &str) -> String {
    let bank = AtomicParamBank::new();
    let mut out = String::with_capacity(16384);

    writeln!(
        out,
        r#"@prefix atom:   <http://lv2plug.in/ns/ext/atom#> .
@prefix bufsz:  <http://lv2plug.in/ns/ext/buf-size#> .
@prefix doap:   <http://usefulinc.com/ns/doap#> .
@prefix foaf:   <http://xmlns.com/foaf/0.1/> .
@prefix lv2:    <http://lv2plug.in/ns/lv2core#> .
@prefix midi:   <http://lv2plug.in/ns/ext/midi#> .
@prefix opts:   <http://lv2plug.in/ns/ext/options#> .
@prefix patch:  <http://lv2plug.in/ns/ext/patch#> .
@prefix pg:     <http://lv2plug.in/ns/ext/port-groups#> .
@prefix pprops: <http://lv2plug.in/ns/ext/port-props#> .
@prefix rdf:    <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
@prefix rsz:    <http://lv2plug.in/ns/ext/resize-port#> .
@prefix state:  <http://lv2plug.in/ns/ext/state#> .
@prefix time:   <http://lv2plug.in/ns/ext/time#> .
@prefix ui:     <http://lv2plug.in/ns/extensions/ui#> .
@prefix units:  <http://lv2plug.in/ns/extensions/units#> .
@prefix urid:   <http://lv2plug.in/ns/ext/urid#> .
@prefix work:   <http://lv2plug.in/ns/ext/worker#> .

<{PLUGIN_URI_STR}#out_group>
	a pg:StereoGroup ,
		pg:OutputGroup ;
	lv2:name "Main Output" ;
	lv2:symbol "main_out" .

<{UI_URI_STR}>
	a ui:X11UI ;
	ui:binary <{binary_filename}> ;
	lv2:optionalFeature ui:parent ,
		ui:resize ,
		<http://lv2plug.in/ns/ext/instance-access> ;
	lv2:extensionData ui:idleInterface ,
		ui:showInterface ,
		ui:resize .

<{PLUGIN_URI_STR}>
	a lv2:Plugin ,
		lv2:InstrumentPlugin ;
	doap:name "PROPHET-XII" ;
	doap:license <https://spdx.org/licenses/GPL-3.0-only> ;
	doap:maintainer [
		foaf:name "PROPHET-XII Contributors"
	] ;
	lv2:minorVersion 1 ;
	lv2:microVersion 0 ;
	pg:mainOutput <{PLUGIN_URI_STR}#out_group> ;
	lv2:requiredFeature urid:map ;
	lv2:optionalFeature lv2:hardRTCapable ,
		work:schedule ,
		opts:options ,
		bufsz:boundedBlockLength ;
	lv2:extensionData state:interface ,
		work:interface ,
		opts:interface ;
	ui:ui <{UI_URI_STR}> ;
	lv2:port ["#,
    )
    .unwrap();

    // Ports 0..3: Stereo audio outputs + Atom control/notify sequences
    writeln!(
        out,
        r#"		a lv2:AudioPort ,
			lv2:OutputPort ;
		lv2:index 0 ;
		lv2:symbol "out_l" ;
		lv2:name "Audio Output L" ;
		pg:group <{PLUGIN_URI_STR}#out_group> ;
		lv2:designation pg:left
	] , [
		a lv2:AudioPort ,
			lv2:OutputPort ;
		lv2:index 1 ;
		lv2:symbol "out_r" ;
		lv2:name "Audio Output R" ;
		pg:group <{PLUGIN_URI_STR}#out_group> ;
		lv2:designation pg:right
	] , [
		a atom:AtomPort ,
			lv2:InputPort ;
		lv2:index 2 ;
		lv2:symbol "control_in" ;
		lv2:name "MIDI / Control In" ;
		atom:bufferType atom:Sequence ;
		atom:supports midi:MidiEvent ,
			time:Position ,
			patch:Message ;
		lv2:designation lv2:control
	] , [
		a atom:AtomPort ,
			lv2:OutputPort ;
		lv2:index 3 ;
		lv2:symbol "notify_out" ;
		lv2:name "Notify Out" ;
		atom:bufferType atom:Sequence ;
		atom:supports atom:Object ,
			patch:Message ,
			time:Position ;
		rsz:minimumSize 8192 ;
		lv2:portProperty lv2:connectionOptional
	] , [
		a lv2:ControlPort ,
			lv2:InputPort ;
		lv2:index 4 ;
		lv2:symbol "reverb_enabled" ;
		lv2:name "Reverb Enabled" ;
		lv2:default 1 ;
		lv2:minimum 0 ;
		lv2:maximum 1 ;
		lv2:portProperty lv2:toggled ,
			pprops:causesArtifacts
	] , [
		a lv2:ControlPort ,
			lv2:InputPort ;
		lv2:index 5 ;
		lv2:symbol "reverb_kill_tail" ;
		lv2:name "Reverb Kill Tail (off = Spill)" ;
		lv2:default 0 ;
		lv2:minimum 0 ;
		lv2:maximum 1 ;
		lv2:portProperty lv2:toggled"#
    )
    .unwrap();

    // Ports 6..88: PARAM_SPECS[0..83]
    for (idx, spec) in PARAM_SPECS.iter().enumerate() {
        let port_idx = PORT_PARAMS_START + idx as u32;
        let def_val = bank.get(spec.id);
        let is_toggled = spec.integer && (spec.min - 0.0).abs() < 1e-6 && (spec.max - 1.0).abs() < 1e-6;
        let scales = scale_points_for_param(spec.id);

        let mut props: Vec<&'static str> = Vec::new();
        if !scales.is_empty() {
            props.push("lv2:enumeration");
        }
        if is_toggled {
            props.push("lv2:toggled");
        } else if spec.integer {
            props.push("lv2:integer");
        }
        if spec.logarithmic {
            props.push("pprops:logarithmic");
        }
        if spec.id == ParamId::OversamplingMode {
            props.push("pprops:expensive");
            props.push("pprops:causesArtifacts");
        }

        writeln!(out, "\t] , [").unwrap();
        writeln!(out, "\t\ta lv2:ControlPort ,").unwrap();
        writeln!(out, "\t\t\tlv2:InputPort ;").unwrap();
        writeln!(out, "\t\tlv2:index {port_idx} ;").unwrap();
        writeln!(out, "\t\tlv2:symbol \"p{idx}\" ;").unwrap();
        writeln!(out, "\t\tlv2:name \"{}\" ;", spec.name).unwrap();
        writeln!(
            out,
            "\t\tlv2:default {} ;",
            fmt_ttl_num(def_val, spec.integer)
        )
        .unwrap();
        writeln!(
            out,
            "\t\tlv2:minimum {} ;",
            fmt_ttl_num(spec.min, spec.integer)
        )
        .unwrap();

        let has_more = !props.is_empty() || unit_for_param(spec.id).is_some() || !scales.is_empty();
        writeln!(
            out,
            "\t\tlv2:maximum {}{}",
            fmt_ttl_num(spec.max, spec.integer),
            if has_more { " ;" } else { "" }
        )
        .unwrap();

        if !props.is_empty() {
            let term = if unit_for_param(spec.id).is_some() || !scales.is_empty() {
                " ;"
            } else {
                ""
            };
            writeln!(out, "\t\tlv2:portProperty {}{}", props.join(" ,\n\t\t\t"), term).unwrap();
        }
        if let Some(u) = unit_for_param(spec.id) {
            let term = if !scales.is_empty() { " ;" } else { "" };
            writeln!(out, "\t\tunits:unit {u}{term}").unwrap();
        }
        if !scales.is_empty() {
            let spts: Vec<String> = scales
                .iter()
                .map(|(lbl, v)| format!("\t\t\t[\n\t\t\t\trdfs:label \"{lbl}\" ;\n\t\t\t\trdf:value {v}\n\t\t\t]"))
                .collect();
            writeln!(out, "\t\tlv2:scalePoint\n{}", spts.join(" ,\n")).unwrap();
        }
    }

    // Port 89: Glide Enabled + Port 90: Latency output
    writeln!(
        out,
        r#"	] , [
		a lv2:ControlPort ,
			lv2:InputPort ;
		lv2:index {PORT_GLIDE_ENABLED} ;
		lv2:symbol "p83" ;
		lv2:name "Glide Enabled" ;
		lv2:default 0 ;
		lv2:minimum 0 ;
		lv2:maximum 1 ;
		lv2:portProperty lv2:toggled
	] , [
		a lv2:ControlPort ,
			lv2:OutputPort ;
		lv2:index {PORT_LATENCY} ;
		lv2:symbol "latency" ;
		lv2:name "Latency" ;
		lv2:default 16 ;
		lv2:minimum 0 ;
		lv2:maximum 1024 ;
		lv2:portProperty lv2:integer ,
			lv2:reportsLatency ;
		lv2:designation lv2:latency ;
		units:unit units:frame
	] ."#
    )
    .unwrap();

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_defines_91_ports_and_required_extensions() {
        let manifest = generate_manifest_ttl("libprophet_lv2.so");
        assert!(manifest.contains(PLUGIN_URI_STR));
        assert!(manifest.contains(UI_URI_STR));
        assert!(manifest.contains("ui:X11UI"));

        let ttl = generate_plugin_ttl("libprophet_lv2.so");
        for idx in 0..TOTAL_PORTS {
            assert!(
                ttl.contains(&format!("lv2:index {idx} ;")),
                "Missing port index {idx} in generated TTL"
            );
        }
        assert!(!ttl.contains("lv2:index 91 ;"));
        assert!(ttl.contains("lv2:reportsLatency"));
        assert!(ttl.contains("state:interface"));
        assert!(ttl.contains("work:interface"));
        assert!(ttl.contains("opts:interface"));
        assert!(ttl.contains("pprops:logarithmic"));
        assert!(ttl.contains("units:hz"));
    }
}
