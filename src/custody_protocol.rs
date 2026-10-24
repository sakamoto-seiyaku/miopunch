//! Bounded custody operations over an authenticated member data connection.
use serde::{Deserialize, Serialize};

use crate::credential_custody::{CustodyOffer, CustodyProbe, CustodyProof, CustodyReceipt};

// No Debug implementation: Deliver is deliberately secret-bearing.
#[derive(Serialize, Deserialize)]
pub enum CustodyRequest {
    Offer { offer: CustodyOffer },
    Deliver { offer: CustodyOffer, seed: [u8; 32] },
    Probe { probe: CustodyProbe },
}
#[derive(Serialize, Deserialize)]
pub struct CustodyReply {
    pub accepted: bool,
    pub receipt: Option<CustodyReceipt>,
    pub proof: Option<CustodyProof>,
}
