use std::collections::{HashMap, HashSet};
use std::sync::{atomic::AtomicBool, Arc};

use parking_lot::RwLock;

use crate::bot::steps::{AuctionStep, BazaarStep, InstaSellStep, SellInventoryStep};

/// All bazaar-specific runtime fields, grouped out of BotClientState.
///
/// The Arc instances here are the same ones held by BotClient — cloned in
/// from the BotClient when BotClientState is constructed inside connect().
#[derive(Clone)]
pub struct BazaarCtx {
    // ---- Order placement context ----
    pub item_name: Arc<RwLock<String>>,
    pub amount: Arc<RwLock<u64>>,
    pub price_per_unit: Arc<RwLock<f64>>,
    pub is_buy_order: Arc<RwLock<bool>>,
    pub step: Arc<RwLock<BazaarStep>>,

    // ---- Flow-specific step trackers ----
    pub insta_sell_item: Arc<RwLock<Option<String>>>,
    pub insta_sell_step: Arc<RwLock<InstaSellStep>>,
    pub sell_inventory_step: Arc<RwLock<SellInventoryStep>>,

    // ---- State flags ----
    pub at_limit: Arc<AtomicBool>,
    pub daily_limit: Arc<AtomicBool>,
    pub order_rejected: Arc<AtomicBool>,

    // ---- ManageOrders state ----
    pub manage_orders_cancel_open: Arc<AtomicBool>,
    pub manage_orders_processed: Arc<RwLock<HashSet<String>>>,
    /// Context of the order currently being managed.
    /// Fields: `(is_buy, display_name, identity (is_buy, item_tag), filled_amount)`
    pub managing_order_context: Arc<RwLock<Option<(bool, String, Option<(bool, String)>, Option<u64>)>>>,
    pub manage_orders_deadline: Arc<RwLock<Option<tokio::time::Instant>>>,
    pub order_cancel_failures: Arc<RwLock<HashMap<String, u32>>>,

    /// Minutes per million coins threshold for age-based order cancellation.
    /// 0 = disabled.
    pub cancel_minutes_per_million: u64,
}

impl BazaarCtx {
    pub fn new(cancel_minutes_per_million: u64) -> Self {
        Self {
            item_name: Arc::new(RwLock::new(String::new())),
            amount: Arc::new(RwLock::new(0)),
            price_per_unit: Arc::new(RwLock::new(0.0)),
            is_buy_order: Arc::new(RwLock::new(true)),
            step: Arc::new(RwLock::new(BazaarStep::Initial)),
            insta_sell_item: Arc::new(RwLock::new(None)),
            insta_sell_step: Arc::new(RwLock::new(InstaSellStep::FindItem)),
            sell_inventory_step: Arc::new(RwLock::new(SellInventoryStep::Initial)),
            at_limit: Arc::new(AtomicBool::new(false)),
            daily_limit: Arc::new(AtomicBool::new(false)),
            order_rejected: Arc::new(AtomicBool::new(false)),
            manage_orders_cancel_open: Arc::new(AtomicBool::new(false)),
            manage_orders_processed: Arc::new(RwLock::new(HashSet::new())),
            managing_order_context: Arc::new(RwLock::new(None)),
            manage_orders_deadline: Arc::new(RwLock::new(None)),
            order_cancel_failures: Arc::new(RwLock::new(HashMap::new())),
            cancel_minutes_per_million,
        }
    }

    /// Clone all Arc fields into a new BazaarCtx, sharing the same underlying data.
    /// Used in connect() to hand the state component the same Arcs as BotClient.
    pub fn clone_arcs(&self) -> Self {
        Self {
            item_name: Arc::clone(&self.item_name),
            amount: Arc::clone(&self.amount),
            price_per_unit: Arc::clone(&self.price_per_unit),
            is_buy_order: Arc::clone(&self.is_buy_order),
            step: Arc::clone(&self.step),
            insta_sell_item: Arc::clone(&self.insta_sell_item),
            insta_sell_step: Arc::clone(&self.insta_sell_step),
            sell_inventory_step: Arc::clone(&self.sell_inventory_step),
            at_limit: Arc::clone(&self.at_limit),
            daily_limit: Arc::clone(&self.daily_limit),
            order_rejected: Arc::clone(&self.order_rejected),
            manage_orders_cancel_open: Arc::clone(&self.manage_orders_cancel_open),
            manage_orders_processed: Arc::clone(&self.manage_orders_processed),
            managing_order_context: Arc::clone(&self.managing_order_context),
            manage_orders_deadline: Arc::clone(&self.manage_orders_deadline),
            order_cancel_failures: Arc::clone(&self.order_cancel_failures),
            cancel_minutes_per_million: self.cancel_minutes_per_million,
        }
    }
}

/// All auction-house-specific runtime fields, grouped out of BotClientState.
#[derive(Clone)]
pub struct AuctionCtx {
    // ---- Auction creation context ----
    pub item_name: Arc<RwLock<String>>,
    pub starting_bid: Arc<RwLock<u64>>,
    pub duration_hours: Arc<RwLock<u64>>,
    pub item_slot: Arc<RwLock<Option<u64>>>,
    pub item_id: Arc<RwLock<Option<String>>>,
    pub step: Arc<RwLock<AuctionStep>>,

    // ---- State flags ----
    pub sell_aborted: Arc<AtomicBool>,
    pub stuck_item_retries: Arc<std::sync::atomic::AtomicU8>,
    pub at_limit: Arc<AtomicBool>,

    // ---- Cancel context ----
    pub cancel_item_name: Arc<RwLock<String>>,
    pub cancel_starting_bid: Arc<RwLock<i64>>,

    /// Active AH listings by lowercase item name, used to filter coop-member sales.
    pub active_listings: Arc<RwLock<HashSet<String>>>,

    /// Cached "My Auctions" JSON shared with BotClient for instant replies.
    pub cached_my_auctions_json: Arc<RwLock<Option<String>>>,
}

impl AuctionCtx {
    pub fn new() -> Self {
        Self {
            item_name: Arc::new(RwLock::new(String::new())),
            starting_bid: Arc::new(RwLock::new(0)),
            duration_hours: Arc::new(RwLock::new(24)),
            item_slot: Arc::new(RwLock::new(None)),
            item_id: Arc::new(RwLock::new(None)),
            step: Arc::new(RwLock::new(AuctionStep::Initial)),
            sell_aborted: Arc::new(AtomicBool::new(false)),
            stuck_item_retries: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            at_limit: Arc::new(AtomicBool::new(false)),
            cancel_item_name: Arc::new(RwLock::new(String::new())),
            cancel_starting_bid: Arc::new(RwLock::new(0)),
            active_listings: Arc::new(RwLock::new(HashSet::new())),
            cached_my_auctions_json: Arc::new(RwLock::new(None)),
        }
    }

    /// Clone all Arc fields into a new AuctionCtx, sharing the same underlying data.
    pub fn clone_arcs(&self) -> Self {
        Self {
            item_name: Arc::clone(&self.item_name),
            starting_bid: Arc::clone(&self.starting_bid),
            duration_hours: Arc::clone(&self.duration_hours),
            item_slot: Arc::clone(&self.item_slot),
            item_id: Arc::clone(&self.item_id),
            step: Arc::clone(&self.step),
            sell_aborted: Arc::clone(&self.sell_aborted),
            stuck_item_retries: Arc::clone(&self.stuck_item_retries),
            at_limit: Arc::clone(&self.at_limit),
            cancel_item_name: Arc::clone(&self.cancel_item_name),
            cancel_starting_bid: Arc::clone(&self.cancel_starting_bid),
            active_listings: Arc::clone(&self.active_listings),
            cached_my_auctions_json: Arc::clone(&self.cached_my_auctions_json),
        }
    }
}
