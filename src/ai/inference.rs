//! Binary-only CPU inference. Never include this module in the TSF library.
#[path = "assets.rs"]
mod assets;

use flower::ai::{eligible_text, valid_correction};
use ort::{
    session::Session,
    tensor::TensorElementType,
    value::{Tensor, ValueType},
};
use std::{collections::HashMap, path::Path};

const VOCAB_SIZE: usize = 21128;
fn failed(_: impl std::fmt::Display) -> String {
    "AI inference failed".into()
}

pub struct Corrector {
    session: Session,
    vocab: Vec<String>,
    ids: HashMap<char, i64>,
}

impl Corrector {
    /// `bundle` is the fixed `ai` directory, not a manifest-selected path.
    pub fn load(bundle: &Path) -> Result<Self, String> {
        let bundle = assets::verify(bundle)?;
        let vocab = std::fs::read_to_string(bundle.join("vocab.txt")).map_err(failed)?;
        let vocab: Vec<String> = vocab.lines().map(str::to_owned).collect();
        if vocab.len() != VOCAB_SIZE
            || vocab[100] != "[UNK]"
            || vocab[101] != "[CLS]"
            || vocab[102] != "[SEP]"
        {
            return Err("AI vocabulary invalid".into());
        }
        let ids = vocab
            .iter()
            .enumerate()
            .filter_map(|(i, token)| {
                let mut chars = token.chars();
                let c = chars.next()?;
                (chars.next().is_none() && ('\u{4e00}'..='\u{9fff}').contains(&c))
                    .then_some((c, i as i64))
            })
            .collect();
        assets::restrict_dll_search()?;
        let runtime = bundle.join("onnxruntime.dll");
        let runtime = runtime.to_str().ok_or("AI runtime path invalid")?;
        // Must precede every ORT API call. No environment/PATH fallback.
        ort::init_from(runtime).commit().map_err(failed)?;
        // Fixed hash pins a self-contained graph. In-memory loading supplies no
        // model directory from which arbitrary external tensor files can resolve.
        let model = std::fs::read(bundle.join("model_int8.onnx")).map_err(failed)?;
        let session = Session::builder()
            .map_err(failed)?
            .with_intra_threads(2)
            .map_err(failed)?
            .with_inter_threads(1)
            .map_err(failed)?
            .commit_from_memory(&model)
            .map_err(failed)?;
        if session.inputs.len() != 3 || session.outputs.len() != 1 {
            return Err("AI graph signature invalid".into());
        }
        for name in ["input_ids", "attention_mask", "token_type_ids"] {
            let input = session
                .inputs
                .iter()
                .find(|i| i.name == name)
                .ok_or("AI graph signature invalid")?;
            if !matches!(&input.input_type, ValueType::Tensor { ty: TensorElementType::Int64, dimensions, .. }
                if dimensions.len() == 2 && matches!(dimensions[0], -1 | 1) && dimensions[1] == -1)
            {
                return Err("AI graph signature invalid".into());
            }
        }
        let output = &session.outputs[0];
        if output.name != "logits"
            || !matches!(&output.output_type, ValueType::Tensor { ty: TensorElementType::Float32, dimensions, .. }
                if dimensions.len() == 3 && matches!(dimensions[0], -1 | 1)
                    && dimensions[1] == -1 && dimensions[2] == VOCAB_SIZE as i64)
        {
            return Err("AI graph signature invalid".into());
        }
        Ok(Self {
            session,
            vocab,
            ids,
        })
    }

    pub fn correct(&mut self, text: &str) -> Result<Option<String>, String> {
        if !eligible_text(text) {
            return Err("AI input ineligible".into());
        }
        let mut ids = vec![101i64];
        for c in text.chars() {
            ids.push(*self.ids.get(&c).ok_or("AI character unsupported")?);
        }
        ids.push(102);
        let len = ids.len();
        let input =
            Tensor::from_array(([1usize, len], ids.clone().into_boxed_slice())).map_err(failed)?;
        let mask = Tensor::from_array(([1usize, len], vec![1i64; len].into_boxed_slice()))
            .map_err(failed)?;
        let types = Tensor::from_array(([1usize, len], vec![0i64; len].into_boxed_slice()))
            .map_err(failed)?;
        let outputs = self
            .session
            .run(
                ort::inputs![
                    "input_ids" => input, "attention_mask" => mask, "token_type_ids" => types
                ]
                .map_err(failed)?,
            )
            .map_err(failed)?;
        let (shape, logits) = outputs["logits"]
            .try_extract_raw_tensor::<f32>()
            .map_err(failed)?;
        filter_logits(text, &ids[1..len - 1], &self.vocab, shape, logits)
    }
}

/// Also usable by deterministic fixtures; it does not initialize ORT.
pub fn filter_logits(
    text: &str,
    ids: &[i64],
    vocab: &[String],
    shape: &[i64],
    logits: &[f32],
) -> Result<Option<String>, String> {
    let len = text.chars().count();
    if !eligible_text(text)
        || ids.len() != len
        || vocab.len() != VOCAB_SIZE
        || shape != [1, (len + 2) as i64, VOCAB_SIZE as i64]
        || logits.len() != (len + 2) * VOCAB_SIZE
        || logits.iter().any(|v| !v.is_finite())
        || ids.iter().any(|&id| !(0..VOCAB_SIZE as i64).contains(&id))
    {
        return Err("AI output invalid".into());
    }
    let mut corrected = String::new();
    for (position, original) in text.chars().enumerate() {
        let row = &logits[(position + 1) * VOCAB_SIZE..(position + 2) * VOCAB_SIZE];
        let (best, &max) = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .ok_or("AI output invalid")?;
        if best == ids[position] as usize {
            corrected.push(original);
            continue;
        }
        let denominator: f64 = row.iter().map(|&v| ((v as f64) - max as f64).exp()).sum();
        if 1.0 / denominator < 0.98 || max - row[ids[position] as usize] < 4.0 {
            corrected.push(original);
            continue;
        }
        let mut chars = vocab[best].chars();
        let c = chars.next().ok_or("AI output invalid")?;
        if chars.next().is_some() || !('\u{4e00}'..='\u{9fff}').contains(&c) {
            corrected.push(original);
            continue;
        }
        corrected.push(c);
    }
    Ok(valid_correction(text, &corrected).then_some(corrected))
}

pub fn probe(bundle: &Path, text: &str) -> Result<Option<String>, String> {
    Corrector::load(bundle)?.correct(text)
}

pub fn run_server() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(failed)?;
    let bundle = exe.parent().ok_or("AI bundle unavailable")?.join("ai");
    let mut corrector = None;
    // serve binds the single-instance pipe before this closure can cold-load.
    flower::ai::ipc::serve(move |text| {
        if corrector.is_none() {
            corrector = Some(Corrector::load(&bundle)?);
        }
        corrector.as_mut().ok_or("AI unavailable")?.correct(text)
    })
}
