//! `ptts`: a voice model run on the CPU or the device's own graphics, compiled in with the `phonon` feature.
//! Everything about a model that is not the program is its row's to say (`with`): the form its numbers are
//! in, the language it speaks, what it runs on.
use super::{ByHost, Engine, Loaded};
use crate::models::Model;
use ptts::checkpoint::{Checkpoint, ResolveOptions};
use ptts::preprocess::Lang;
use ptts::synth::{DeviceKind, Quant, SpeechOptions, Synth};
use std::path::Path;

pub(super) struct Ptts;

/// What a row calls each form of a model's numbers, each language, and each thing to run it on.
const QUANTS: &[(&str, Quant)] = &[("f32", Quant::F32), ("q80", Quant::Q80), ("q81", Quant::Q81), ("q8k", Quant::Q8k), ("q6k", Quant::Q6k), ("q50", Quant::Q50), ("q51", Quant::Q51)];
const LANGS: &[(&str, Lang)] = &[("en", Lang::En), ("fr", Lang::Fr), ("de", Lang::De), ("es", Lang::Es), ("pt", Lang::Pt)];
const DEVICES: &[(&str, DeviceKind)] = &[("cpu", DeviceKind::Cpu), ("auto", DeviceKind::Auto)];

/// What the row says for `field`, as one of the engine's own values. A row that does not say, or says
/// something this build does not know, does not load: nothing is assumed for it.
fn told<T: Copy>(m: &Model, field: &str, known: &[(&str, T)]) -> Result<T, String> {
    let said = m.told(field);
    known.iter().find(|(name, _)| *name == said).map(|(_, v)| *v).ok_or_else(|| if said.is_empty() { format!("its row does not say its {field}") } else { format!("this version of airband does not know the {field} \"{said}\"") })
}

impl Engine for Ptts {
    fn load(&self, folder: &Path, m: &Model) -> Result<Box<dyn Loaded>, String> {
        let (quant, lang, device) = (told(m, "quant", QUANTS)?, told(m, "lang", LANGS)?, told(m, "device", DEVICES)?);
        let checkpoint = Checkpoint::resolve(folder, ResolveOptions { quant, weights: None }).map_err(|e| e.to_string())?;
        let mut synth = checkpoint.builder(lang).device(device).build().map_err(|e| e.to_string())?;
        checkpoint.register_voices(&mut synth);
        Ok(Box::new(Voice(synth)))
    }
}

struct Voice(Synth);

impl Loaded for Voice {
    fn speak(&mut self, _: ByHost, voice: &str, text: &str, sink: &mut dyn FnMut(&[u8]), stop: &dyn Fn() -> bool) -> Result<(), String> {
        let stream = self.0.stream_with(text, &SpeechOptions::default().voice(voice.to_string())).map_err(|e| e.to_string())?;
        for chunk in stream {
            if stop() {
                break;
            }
            let chunk = chunk.map_err(|e| e.to_string())?;
            let pcm: Vec<u8> = chunk.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
            sink(&pcm);
        }
        Ok(())
    }
    fn hear(&mut self, _: ByHost, _: Vec<u8>) -> Result<String, String> {
        Err("this model speaks and does not listen".into())
    }
}
