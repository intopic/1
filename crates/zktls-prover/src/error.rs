use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProverError {
    #[error("JSON i pavlefshëm: {0}")]
    Json(#[from] serde_json::Error),
    #[error("hex i pavlefshëm: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("certifikatë/çelës i pavlefshëm: {0}")]
    Key(String),
    #[error("normalizer: {0}")]
    Normalizer(#[from] triad_core::NormalizerError),
    #[error("atestim i pavlefshëm: {0}")]
    Attestation(String),
    #[error("kushti nuk plotësohet: vlera është jashtë diapazonit [min, max]")]
    ThresholdNotMet,
    #[error("Groth16: {0}")]
    Groth16(String),
    #[error("snarkjs: {0}")]
    Snarkjs(String),
    #[error("kodim: {0}")]
    Encoding(String),
}

impl From<ark_relations::r1cs::SynthesisError> for ProverError {
    fn from(e: ark_relations::r1cs::SynthesisError) -> Self {
        ProverError::Groth16(e.to_string())
    }
}
