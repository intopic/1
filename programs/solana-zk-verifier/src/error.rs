use anchor_lang::prelude::*;

#[error_code]
pub enum ZkVerifierError {
    #[msg("Prova Groth16 nuk është e vlefshme për këto hyrje publike")]
    InvalidProof,
    #[msg("Numri i hyrjeve publike nuk përputhet me çelësin verifikues")]
    PublicInputsLength,
    #[msg("Hyrja publike nuk është element i fushës skalare BN254 (>= r)")]
    PublicInputOutOfField,
    #[msg("Pikë e pavlefshme në kurbën BN254")]
    InvalidPoint,
    #[msg("Thirrja sistemore alt_bn128 dështoi")]
    Bn254SyscallFailed,
    #[msg("Çelësi publik i bankës/noterit nuk është në listën e bardhë")]
    BankKeyNotWhitelisted,
    #[msg("Lista e bardhë është plot")]
    WhitelistFull,
    #[msg("Çelësi është tashmë në listën e bardhë")]
    BankKeyAlreadyWhitelisted,
    #[msg("Prova nuk është lidhur me portofolin e nënshkruesit")]
    WalletMismatch,
    #[msg("Atestimi ka skaduar ose ka kohë në të ardhmen")]
    StaleAttestation,
    #[msg("Llogaria burimore është përdorur tashmë nga një portofol tjetër (nullifier)")]
    NullifierAlreadyUsed,
    #[msg("Argumenti nullifier nuk përputhet me hyrjen publike")]
    NullifierMismatch,
    #[msg("Prova është për një app_id tjetër")]
    AppIdMismatch,
    #[msg("Çelësi verifikues ka shumë pika IC")]
    VerifyingKeyTooLarge,
}
