//! The provider table, which the phone core shares (see `airband_core::providers`), as this program sees
//! it: the offers it has the code for. Reading is done here in the tower (`tower/judge.rs`); speaking and
//! listening by the phone core, which the tower uses too.
pub use airband_core::providers::*;
use std::sync::Arc;

/// Does this program have the code for what this offer names?
pub fn speaks(offer: &Offer) -> bool {
    match offer.job {
        Job::Summary => crate::tower::judge::reads(offer),
        Job::Voice => airband_radio::voices::speaks(offer),
        Job::Ears => airband_radio::hears::listens(offer),
    }
}

/// The providers he can choose for a job on this computer: those the catalogue has that this build can use.
pub fn offering(job: Job) -> Vec<Arc<Provider>> {
    airband_core::providers::offering(job).into_iter().filter(|p| p.offers.iter().any(|o| o.job == job && speaks(o))).collect()
}

/// The same, as offers in the order to show them.
pub fn in_order(job: Job) -> Vec<(Arc<Provider>, Arc<Offer>)> {
    airband_core::providers::in_order(job).into_iter().filter(|(_, o)| speaks(o)).collect()
}
