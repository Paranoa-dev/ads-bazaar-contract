use ads_bazaar_shared::{ApplicationStatus, CampaignId, CampaignStatus, PayoutAsset};
use soroban_sdk::{contracttype, Address, String};

/// A creator campaign funded and escrowed by a single business.
///
/// `escrow_balance` is tracked separately from `total_budget` so partial
/// releases (once implemented) can be reconciled against what is actually
/// still held by the contract, independent of what was originally deposited.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Campaign {
    pub id: CampaignId,
    pub business: Address,
    pub asset: PayoutAsset,
    pub total_budget: i128,
    pub escrow_balance: i128,
    /// Sum of payout amounts committed to selected (not-yet-paid) creators.
    /// Reserved against `escrow_balance` so total commitments never exceed funds.
    pub committed_payouts: i128,
    pub max_creators: u32,
    pub approved_count: u32,
    /// Ledger timestamp (unix seconds) after which new applications are rejected.
    pub application_deadline: u64,
    /// Ledger timestamp (unix seconds) by which approved creators must submit proof.
    pub completion_deadline: u64,
    /// Off-chain pointer (IPFS/HTTPS URI) to the full campaign brief.
    pub metadata_uri: String,
    pub status: CampaignStatus,
}

/// A single creator's application to a campaign.
///
/// TODO(contributors): `payout_amount` is set at approval time in the current
/// design sketch (business decides per-creator pay when approving). Revisit
/// if campaigns should instead split `total_budget` evenly across
/// `max_creators`, or support tiered/milestone payouts.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Application {
    pub campaign_id: CampaignId,
    pub creator: Address,
    pub pitch_uri: String,
    pub proof_uri: Option<String>,
    pub payout_amount: i128,
    /// Whether the business has accepted the submitted proof (making it payable).
    pub proof_approved: bool,
    pub status: ApplicationStatus,
}

/// Snapshot of protocol-level configuration, returned by
/// `get_protocol_config` so the frontend can compute fee breakdowns before a
/// business funds a campaign.
///
/// `treasury` defaults to `admin` at `initialize` time — there is no
/// separate fee-collection destination yet (see the TODO on
/// `release_payment` in `lib.rs`). A future issue can add a
/// `set_treasury` admin-only setter if/when that needs to diverge.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolConfig {
    pub admin: Address,
    pub treasury: Address,
    pub fee_bps: i128,
}
