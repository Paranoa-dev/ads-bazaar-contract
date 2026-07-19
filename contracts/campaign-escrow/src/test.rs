//! Comprehensive test suite for the campaign-escrow contract.
//!
//! Covers the full lifecycle (create → fund → apply → select → submit proof →
//! approve → claim), fee calculation, cancellation, surplus reclaim, expiry,
//! plus auth and deadline enforcement. Helpers live in `test_helpers` so the
//! individual test modules stay focused on assertions.
#![cfg(test)]

mod test_helpers {
    use crate::{CampaignEscrowContract, CampaignEscrowContractClient, PayoutAsset};
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::token::StellarAssetClient;
    use soroban_sdk::{Address, Env, String};

    /// Base ledger timestamp all tests start from (so deadlines are relative
    /// and controllable via `advance_time` / direct assignment).
    pub const BASE_TIME: u64 = 1_000_000;
    /// Amount minted to the business so it can fund campaigns.
    pub const BUSINESS_FUNDS: i128 = 1_000_000_000;

    /// Register the contract with `mock_all_auths` enabled and a fixed base
    /// timestamp. Returns `(env, contract_id)`.
    pub fn setup_env() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.timestamp = BASE_TIME);
        let contract_id = env.register(CampaignEscrowContract, ());
        (env, contract_id)
    }

    /// Register a Stellar Asset Contract, mint `amount` to `mint_to`, and
    /// return the token address.
    pub fn setup_token(env: &Env, mint_to: &Address, amount: i128) -> Address {
        let admin = Address::generate(env);
        let token = env.register_stellar_asset_contract_v2(admin);
        let token_address = token.address();
        let sac = StellarAssetClient::new(env, &token_address);
        sac.mint(mint_to, &amount);
        token_address
    }

    /// Advance the ledger timestamp by `seconds`.
    pub fn advance_time(env: &Env, seconds: u64) {
        env.ledger().with_mut(|l| l.timestamp += seconds);
    }

    /// Build a USDC `PayoutAsset` pointing at `token`.
    pub fn usdc(env: &Env, token: &Address) -> PayoutAsset {
        PayoutAsset {
            token: token.clone(),
            symbol: String::from_str(env, "USDC"),
        }
    }

    /// Initialize the contract (admin + dispute contract + fee_bps) and mint
    /// `BUSINESS_FUNDS` to a freshly generated business address. Returns the
    /// client plus the generated identities.
    pub fn bootstrap<'a>(
        env: &'a Env,
        contract_id: &Address,
        fee_bps: i128,
    ) -> (
        CampaignEscrowContractClient<'a>,
        Address,
        Address,
        Address,
        Address,
    ) {
        let client = CampaignEscrowContractClient::new(env, contract_id);
        let admin = Address::generate(env);
        let dispute = Address::generate(env);
        let business = Address::generate(env);
        client.initialize(&admin, &dispute, &fee_bps);
        let token = setup_token(env, &business, BUSINESS_FUNDS);
        (client, admin, dispute, business, token)
    }

    /// Create a campaign and immediately fund it (Draft → Funded), returning
    /// the campaign id.
    pub fn create_funded_campaign(
        env: &Env,
        client: &CampaignEscrowContractClient,
        business: &Address,
        token: &Address,
        total_budget: i128,
        max_creators: u32,
    ) -> u64 {
        let now = env.ledger().timestamp();
        let asset = usdc(env, token);
        let id = client.create_campaign(
            business,
            &asset,
            &total_budget,
            &max_creators,
            &(now + 86_400),
            &(now + 604_800),
            &String::from_str(env, "ipfs://brief"),
        );
        client.fund_campaign(business, &id);
        id
    }
}

mod test_initialize {
    use crate::{CampaignEscrowContractClient, Error};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    #[test]
    fn initialize_sets_admin_and_fee() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(crate::CampaignEscrowContract, ());
        let client = CampaignEscrowContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let dispute = Address::generate(&env);
        client.initialize(&admin, &dispute, &250);
    }

    #[test]
    fn initialize_twice_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(crate::CampaignEscrowContract, ());
        let client = CampaignEscrowContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let dispute = Address::generate(&env);
        client.initialize(&admin, &dispute, &250);
        let result = client.try_initialize(&admin, &dispute, &250);
        assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
    }

    #[test]
    fn initialize_rejects_out_of_range_fee() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(crate::CampaignEscrowContract, ());
        let client = CampaignEscrowContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let dispute = Address::generate(&env);
        let result = client.try_initialize(
            &admin,
            &dispute,
            &(ads_bazaar_shared::BASIS_POINTS_DENOMINATOR + 1),
        );
        assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    }

    #[test]
    fn get_campaign_not_found_before_creation() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(crate::CampaignEscrowContract, ());
        let client = CampaignEscrowContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let dispute = Address::generate(&env);
        client.initialize(&admin, &dispute, &250);
        let result = client.try_get_campaign(&0);
        assert_eq!(result, Err(Ok(Error::CampaignNotFound)));
    }
}

mod test_happy_path {
    use super::test_helpers::*;
    use crate::CampaignEscrowContractClient;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::token::Client as TokenClient;
    use soroban_sdk::{Address, Env};

    /// Drive a campaign to the point where `creator` has an approved,
    /// payable submission: applied → selected → proof submitted → approved.
    fn run_to_payable(
        env: &Env,
        client: &CampaignEscrowContractClient,
        business: &Address,
        creator: &Address,
        id: &u64,
        payout: i128,
    ) {
        client.apply_to_campaign(creator, id, &soroban_sdk::String::from_str(env, "pitch"));
        client.approve_creator(business, id, creator, &payout);
        client.submit_proof(creator, id, &soroban_sdk::String::from_str(env, "proof"));
        client.approve_submission(business, id, creator);
    }

    #[test]
    fn full_lifecycle() {
        let (env, contract_id) = setup_env();
        let (client, admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let token_client = TokenClient::new(&env, &token);

        let creator = Address::generate(&env);
        let gross: i128 = 1_000_000;
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        run_to_payable(&env, &client, &business, &creator, &id, gross);

        let creator_before = token_client.balance(&creator);
        let treasury_before = token_client.balance(&admin);

        client.claim_payment(&creator, &id);

        let fee = gross * 50 / ads_bazaar_shared::BASIS_POINTS_DENOMINATOR;
        let net = gross - fee;
        assert_eq!(token_client.balance(&creator), creator_before + net);
        assert_eq!(token_client.balance(&admin), treasury_before + fee);
    }

    #[test]
    fn fee_calculation_50bps() {
        let (env, contract_id) = setup_env();
        let (client, admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let token_client = TokenClient::new(&env, &token);

        let creator = Address::generate(&env);
        let gross: i128 = 2_000_000;
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        run_to_payable(&env, &client, &business, &creator, &id, gross);
        client.claim_payment(&creator, &id);

        let fee = gross * 50 / ads_bazaar_shared::BASIS_POINTS_DENOMINATOR;
        let net = gross - fee;
        // creator_net == gross * 0.995
        assert_eq!(net, gross * 995 / 1_000);
        // treasury == gross * 0.005
        assert_eq!(fee, gross * 5 / 1_000);
        assert_eq!(token_client.balance(&creator), net);
        assert_eq!(token_client.balance(&admin), fee);
    }

    #[test]
    fn auto_approve_past_deadline() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let token_client = TokenClient::new(&env, &token);

        let creator = Address::generate(&env);
        let gross: i128 = 1_000_000;
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        client.apply_to_campaign(&creator, &id, &soroban_sdk::String::from_str(&env, "pitch"));
        client.approve_creator(&business, &id, &creator, &gross);
        // submit before the deadline (still pending business approval)...
        client.submit_proof(&creator, &id, &soroban_sdk::String::from_str(&env, "proof"));
        // ...then move past the content deadline so it auto-approves.
        advance_time(&env, 604_800 + 10);

        let creator_before = token_client.balance(&creator);
        // Claim without an explicit approve_submission call.
        client.claim_payment(&creator, &id);
        let fee = gross * 50 / ads_bazaar_shared::BASIS_POINTS_DENOMINATOR;
        assert_eq!(token_client.balance(&creator), creator_before + gross - fee);
    }

    #[test]
    fn cancel_open_campaign() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let token_client = TokenClient::new(&env, &token);

        let business_before = token_client.balance(&business);
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        client.cancel_campaign(&business, &id);
        // Business balance is fully restored (no commitments outstanding).
        assert_eq!(token_client.balance(&business), business_before);
        assert_eq!(token_client.balance(&contract_id), 0);
    }

    #[test]
    fn reclaim_surplus_after_payouts() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 0);
        let token_client = TokenClient::new(&env, &token);

        let payout: i128 = 1_000_000;
        // max 5, budget covers 5 payouts of `payout`.
        let id = create_funded_campaign(&env, &client, &business, &token, payout * 5, 5);

        let c1 = Address::generate(&env);
        let c2 = Address::generate(&env);
        for c in [&c1, &c2] {
            client.apply_to_campaign(c, &id, &soroban_sdk::String::from_str(&env, "pitch"));
            client.approve_creator(&business, &id, c, &payout);
            client.submit_proof(c, &id, &soroban_sdk::String::from_str(&env, "proof"));
            client.approve_submission(&business, &id, c);
            client.claim_payment(c, &id);
        }

        let business_before = token_client.balance(&business);
        client.reclaim_surplus(&business, &id);
        // Surplus == (5 - 2) * payout == 3 * payout_per_creator.
        assert_eq!(
            token_client.balance(&business),
            business_before + payout * 3
        );
        assert_eq!(token_client.balance(&contract_id), 0);
    }

    #[test]
    fn reject_and_resubmit() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 0);
        let token_client = TokenClient::new(&env, &token);

        let creator = Address::generate(&env);
        let gross: i128 = 1_000_000;
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        client.apply_to_campaign(&creator, &id, &soroban_sdk::String::from_str(&env, "pitch"));
        client.approve_creator(&business, &id, &creator, &gross);
        client.submit_proof(
            &creator,
            &id,
            &soroban_sdk::String::from_str(&env, "proof-v1"),
        );
        // Business rejects the proof.
        client.reject_submission(&business, &id, &creator);
        // Creator resubmits.
        client.submit_proof(
            &creator,
            &id,
            &soroban_sdk::String::from_str(&env, "proof-v2"),
        );
        client.approve_submission(&business, &id, &creator);

        let creator_before = token_client.balance(&creator);
        client.claim_payment(&creator, &id);
        assert_eq!(token_client.balance(&creator), creator_before + gross);
    }

    #[test]
    fn expire_no_submissions() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let token_client = TokenClient::new(&env, &token);

        let budget: i128 = 10_000_000;
        let id = create_funded_campaign(&env, &client, &business, &token, budget, 5);

        let c1 = Address::generate(&env);
        let c2 = Address::generate(&env);
        // Two creators are selected (committing 1_000_000 each) but never
        // submit proof — their payouts stay reserved against escrow.
        for c in [&c1, &c2] {
            client.apply_to_campaign(c, &id, &soroban_sdk::String::from_str(&env, "pitch"));
            client.approve_creator(&business, &id, c, &1_000_000);
        }

        // Advance past the content deadline.
        advance_time(&env, 604_800 + 10);

        let business_before = token_client.balance(&business);
        client.expire_campaign(&business, &id);
        // Only the unallocated balance (budget - committed) is refunded.
        let committed = 1_000_000 * 2;
        assert_eq!(
            token_client.balance(&business),
            business_before + budget - committed
        );
        // Reserved funds remain in the contract for the selected creators.
        assert_eq!(token_client.balance(&contract_id), committed);
    }

    // --- Fund-safety regression tests: committed payouts must survive
    // cancel / expire / reclaim and remain claimable by the creator. ---

    #[test]
    fn cancel_preserves_committed_payout() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 0);
        let token_client = TokenClient::new(&env, &token);

        let payout: i128 = 1_000_000;
        let budget: i128 = payout * 5;
        let id = create_funded_campaign(&env, &client, &business, &token, budget, 5);

        let creator = Address::generate(&env);
        run_to_payable(&env, &client, &business, &creator, &id, payout);

        // Business cancels before the creator claims.
        let business_before = token_client.balance(&business);
        client.cancel_campaign(&business, &id);
        // Business recovers only the unallocated (budget - payout) portion.
        assert_eq!(
            token_client.balance(&business),
            business_before + budget - payout
        );
        // The committed payout is still held by the contract.
        assert_eq!(token_client.balance(&contract_id), payout);

        // The approved creator can still claim their payout after cancellation.
        let creator_before = token_client.balance(&creator);
        client.claim_payment(&creator, &id);
        assert_eq!(token_client.balance(&creator), creator_before + payout);
        assert_eq!(token_client.balance(&contract_id), 0);
    }

    #[test]
    fn expire_preserves_committed_payout() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 0);
        let token_client = TokenClient::new(&env, &token);

        let payout: i128 = 1_000_000;
        let budget: i128 = payout * 5;
        let id = create_funded_campaign(&env, &client, &business, &token, budget, 5);

        let creator = Address::generate(&env);
        run_to_payable(&env, &client, &business, &creator, &id, payout);

        // Advance past the content deadline, then expire.
        advance_time(&env, 604_800 + 10);
        let business_before = token_client.balance(&business);
        client.expire_campaign(&business, &id);
        assert_eq!(
            token_client.balance(&business),
            business_before + budget - payout
        );
        assert_eq!(token_client.balance(&contract_id), payout);

        // Creator still gets paid after expiry.
        let creator_before = token_client.balance(&creator);
        client.claim_payment(&creator, &id);
        assert_eq!(token_client.balance(&creator), creator_before + payout);
        assert_eq!(token_client.balance(&contract_id), 0);
    }

    #[test]
    fn reclaim_preserves_committed_payout() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 0);
        let token_client = TokenClient::new(&env, &token);

        let payout: i128 = 1_000_000;
        let budget: i128 = payout * 5;
        let id = create_funded_campaign(&env, &client, &business, &token, budget, 5);

        let creator = Address::generate(&env);
        run_to_payable(&env, &client, &business, &creator, &id, payout);

        // Business reclaims surplus before the creator claims.
        let business_before = token_client.balance(&business);
        client.reclaim_surplus(&business, &id);
        assert_eq!(
            token_client.balance(&business),
            business_before + budget - payout
        );
        assert_eq!(token_client.balance(&contract_id), payout);

        // Creator still gets paid after the reclaim.
        let creator_before = token_client.balance(&creator);
        client.claim_payment(&creator, &id);
        assert_eq!(token_client.balance(&creator), creator_before + payout);
        assert_eq!(token_client.balance(&contract_id), 0);
    }
}

mod test_protocol_config {
    use super::test_helpers::setup_env;
    use crate::{CampaignEscrowContractClient, Error};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;

    #[test]
    fn get_protocol_config_returns_current_fee_bps() {
        let (env, contract_id) = setup_env();
        let client = CampaignEscrowContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let dispute_contract = Address::generate(&env);
        client.initialize(&admin, &dispute_contract, &150);

        let config = client.get_protocol_config();
        assert_eq!(config.fee_bps, 150);
        assert_eq!(config.admin, admin);
        // treasury defaults to admin — see the comment on `initialize` in lib.rs
        assert_eq!(config.treasury, admin);
    }

    #[test]
    fn get_protocol_config_fails_before_initialization() {
        let (env, contract_id) = setup_env();
        let client = CampaignEscrowContractClient::new(&env, &contract_id);

        let result = client.try_get_protocol_config();
        assert_eq!(result, Err(Ok(Error::NotInitialized)));
    }
}

mod test_auth_failures {
    use super::test_helpers::*;
    use crate::Error;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, String};

    #[test]
    fn non_owner_cancel() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        let stranger = Address::generate(&env);
        let result = client.try_cancel_campaign(&stranger, &id);
        assert_eq!(result, Err(Ok(Error::NotCampaignOwner)));
    }

    #[test]
    fn non_owner_select_creator() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        let creator = Address::generate(&env);
        let stranger = Address::generate(&env);
        let result = client.try_approve_creator(&stranger, &id, &creator, &1_000_000);
        assert_eq!(result, Err(Ok(Error::NotCampaignOwner)));
    }

    #[test]
    fn non_owner_approve_submission() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        let creator = Address::generate(&env);
        client.apply_to_campaign(&creator, &id, &String::from_str(&env, "pitch"));
        client.approve_creator(&business, &id, &creator, &1_000_000);
        client.submit_proof(&creator, &id, &String::from_str(&env, "proof"));

        let stranger = Address::generate(&env);
        let result = client.try_approve_submission(&stranger, &id, &creator);
        assert_eq!(result, Err(Ok(Error::NotCampaignOwner)));
    }

    #[test]
    fn creator_claim_before_approval() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        let creator = Address::generate(&env);
        client.apply_to_campaign(&creator, &id, &String::from_str(&env, "pitch"));
        client.approve_creator(&business, &id, &creator, &1_000_000);
        client.submit_proof(&creator, &id, &String::from_str(&env, "proof"));

        // Proof is submitted but not yet approved by the business.
        let result = client.try_claim_payment(&creator, &id);
        assert_eq!(result, Err(Ok(Error::SubmissionNotPayable)));
    }

    #[test]
    fn double_apply_same_creator() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        let creator = Address::generate(&env);
        client.apply_to_campaign(&creator, &id, &String::from_str(&env, "pitch"));
        let result = client.try_apply_to_campaign(&creator, &id, &String::from_str(&env, "pitch2"));
        assert_eq!(result, Err(Ok(Error::AlreadyApplied)));
    }

    #[test]
    fn double_select_same_creator() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        let creator = Address::generate(&env);
        client.apply_to_campaign(&creator, &id, &String::from_str(&env, "pitch"));
        client.approve_creator(&business, &id, &creator, &1_000_000);
        let result = client.try_approve_creator(&business, &id, &creator, &1_000_000);
        assert_eq!(result, Err(Ok(Error::AlreadySelected)));
    }
}

mod test_deadline_enforcement {
    use super::test_helpers::*;
    use crate::Error;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, String};

    #[test]
    fn apply_after_application_deadline() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);

        let now = env.ledger().timestamp();
        let asset = usdc(&env, &token);
        let id = client.create_campaign(
            &business,
            &asset,
            &10_000_000,
            &5,
            &(now + 86_400),
            &(now + 604_800),
            &String::from_str(&env, "ipfs://brief"),
        );
        client.fund_campaign(&business, &id);

        // Move past the application deadline.
        advance_time(&env, 86_400 + 10);

        let creator = Address::generate(&env);
        let result = client.try_apply_to_campaign(&creator, &id, &String::from_str(&env, "pitch"));
        assert_eq!(result, Err(Ok(Error::ApplicationDeadlinePassed)));
    }

    #[test]
    fn submit_after_content_deadline() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        let creator = Address::generate(&env);
        client.apply_to_campaign(&creator, &id, &String::from_str(&env, "pitch"));
        client.approve_creator(&business, &id, &creator, &1_000_000);

        // Move past the content deadline.
        advance_time(&env, 604_800 + 10);

        let result = client.try_submit_proof(&creator, &id, &String::from_str(&env, "proof"));
        assert_eq!(result, Err(Ok(Error::ContentDeadlinePassed)));
    }

    #[test]
    fn create_with_past_deadline() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);

        let now = env.ledger().timestamp();
        let asset = usdc(&env, &token);
        let result = client.try_create_campaign(
            &business,
            &asset,
            &10_000_000,
            &5,
            &(now - 100),
            &(now + 604_800),
            &String::from_str(&env, "ipfs://brief"),
        );
        assert_eq!(result, Err(Ok(Error::DeadlineInPast)));
    }

    #[test]
    fn expire_before_deadline() {
        let (env, contract_id) = setup_env();
        let (client, _admin, _dispute, business, token) = bootstrap(&env, &contract_id, 50);
        let id = create_funded_campaign(&env, &client, &business, &token, 10_000_000, 5);

        // Still before the content deadline.
        let result = client.try_expire_campaign(&business, &id);
        assert_eq!(result, Err(Ok(Error::DeadlineNotReached)));
    }
}
