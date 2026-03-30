/// Connection wait duration (seconds) - time to wait for bot connection to establish
pub const CONNECTION_WAIT_SECONDS: u64 = 2;

/// Delay after spawning in lobby before sending /play sb command
pub const LOBBY_COMMAND_DELAY_SECS: u64 = 3;

/// Delay after detecting SkyBlock join before teleporting to island
pub const ISLAND_TELEPORT_DELAY_SECS: u64 = 2;

/// Wait time for island teleport to complete
pub const TELEPORT_COMPLETION_WAIT_SECS: u64 = 3;

/// Timeout for waiting for SkyBlock join confirmation (seconds)
pub const SKYBLOCK_JOIN_TIMEOUT_SECS: u64 = 15;

/// Delay before clicking accept button in trade response window (milliseconds)
/// TypeScript waits to check for "Deal!" or "Warning!" messages before accepting
pub const TRADE_RESPONSE_DELAY_MS: u64 = 3400;
pub const STARTUP_ENTRY_TIMEOUT_SECS: u64 = 60;
/// Interval for safety retry clicks in the Confirm Purchase window (milliseconds).
pub const CONFIRM_PURCHASE_RETRY_MS: u64 = 50;
/// Brief delay after closing a stale window so Hypixel processes the
/// container-close packet before the next command is sent.
pub const WINDOW_CLOSE_DELAY_MS: u64 = 150;
pub const MAX_CLAIM_SOLD_UUID_QUEUE: usize = 64;
/// Delay before retrying the auction flow after closing a window to remove a
/// stuck item from the auction slot.  Gives Hypixel time to process the
/// container-close packet and return the item to inventory.
pub const AUCTION_RETRY_AFTER_STUCK_ITEM_MS: u64 = 2000;
/// Maximum number of retry attempts when "You already have an item in the auction
/// slot!" keeps recurring.  After this many retries the bot gives up and goes Idle
/// instead of looping indefinitely and risking a "Sending packets too fast!" kick.
pub const MAX_AUCTION_STUCK_ITEM_RETRIES: u8 = 3;
/// Fallback slot index for "Manage Orders" in the Bazaar GUI when dynamic name
/// lookup fails.  Hypixel's default layout places it at slot 50.
pub const MANAGE_ORDERS_FALLBACK_SLOT: usize = 50;
/// Fallback slot index for "Sell Inventory Now" in the Bazaar GUI when dynamic
/// name lookup fails.  Hypixel's default layout places it at slot 47.
pub const SELL_INVENTORY_NOW_FALLBACK_SLOT: usize = 47;
/// Debounce interval for `rebuild_cached_window_json` on `ContainerSetSlot` events.
/// Individual slot updates are coalesced within this window to avoid excessive CPU
/// from repeated NBT extraction + JSON serialisation during rapid GUI interactions.
pub const WINDOW_CACHE_REBUILD_DEBOUNCE_MS: u64 = 100;
/// Debounce interval for `rebuild_cached_inventory_json` on `ContainerSetSlot` events.
/// Same rationale as `WINDOW_CACHE_REBUILD_DEBOUNCE_MS`: coalesces rapid per-slot
/// updates into a single rebuild to keep the ECS World lock acquisition frequency low
/// and reduce CPU spent on repeated JSON serialisation.
pub const INVENTORY_CACHE_REBUILD_DEBOUNCE_MS: u64 = 100;
/// Timeout (seconds) for `wait_for_collect_confirmation` and
/// `wait_for_cancel_confirmation` to consider an action unprocessed.
/// Raised from 5 → 8 to accommodate Hypixel server lag that caused
/// frequent false "not confirmed" warnings and skipped events.
pub const ORDER_ACTION_CONFIRMATION_TIMEOUT_SECS: u64 = 8;
/// Maximum number of cancel attempts per order before giving up.
/// After this many failed cancel clicks in Order options, the order is skipped
/// so the bot doesn't get stuck retrying indefinitely.
pub const MAX_CANCEL_RETRIES: u32 = 5;
/// Minimum number of empty player-inventory slots required to consider the
/// inventory "not full".  Used when verifying the `inventory_full` flag
/// against actual slot counts so stale flags are auto-cleared after a manual
/// instasell or any other action that frees space.
pub const MIN_FREE_SLOTS_FOR_BUY: u8 = 2;

