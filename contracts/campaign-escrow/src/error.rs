use soroban_sdk::contracterror;

/// Errors returned by the campaign-escrow contract.
///
/// TODO(contributors): extend as apply/approve/proof/dispute logic is filled
/// in — e.g. `ApplicationAlreadyExists`, `ProofAlreadySubmitted`,
/// `NotApprovedCreator`, `DisputeAlreadyRaised`.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    CampaignNotFound = 4,
    ApplicationNotFound = 5,
    InvalidStatus = 6,
    InvalidAmount = 7,
    DeadlinePassed = 8,
    MaxCreatorsReached = 9,
    InsufficientEscrowBalance = 10,
    /// Caller is not the campaign owner (business) that created the campaign.
    NotCampaignOwner = 11,
    /// A submission/claim was attempted that is not yet eligible for payout.
    SubmissionNotPayable = 12,
    /// The creator has already applied to this campaign.
    AlreadyApplied = 13,
    /// The creator has already been selected (approved) for this campaign.
    AlreadySelected = 14,
    /// Applications are no longer accepted (application deadline passed).
    ApplicationDeadlinePassed = 15,
    /// Proof of work can no longer be submitted (content deadline passed).
    ContentDeadlinePassed = 16,
    /// A deadline was supplied that is in the past.
    DeadlineInPast = 17,
    /// The campaign is not yet past its content deadline.
    DeadlineNotReached = 18,
}
