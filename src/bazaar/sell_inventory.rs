use azalea::prelude::*;
use azalea_protocol::packets::game::s_sign_update::ServerboundSignUpdate;
use azalea_protocol::packets::game::s_container_close::ServerboundContainerClose;
use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::{info, error, debug, warn};
use crate::types::BotState;
use crate::bot::client::{
    BotClientState, BotEvent,
    // step enums
    BazaarStep, InstaSellStep, SellInventoryStep, AuctionStep, CookieStep,
    // utility fns (pub(crate) in client.rs)
    click_window_slot, send_chat_command, send_raw_chat_command,
    send_raw_close, send_raw_click,
    find_slot_by_name, find_slot_by_lore_contains,
    get_item_display_name_from_slot, get_item_lore_from_slot,
    remove_mc_colors, lore_contains_phrase,
    format_price_for_sign,
    is_bazaar_orders_window_title, is_my_auctions_window_title,
    is_order_claimable_from_lore, is_claimable_auction_slot,
    parse_bazaar_filled_notification, parse_filled_amount_from_lore,
    parse_bazaar_order_identity, parse_bazaar_order_identity_from_name,
    is_bazaar_order_entry_name, is_buy_bazaar_order_name,
    should_treat_as_bazaar_order_slot, clean_order_item_name,
    log_bazaar_order_placed, should_cancel_open_order_due_to_age,
    log_pending_claim, check_manage_orders_deadline,
    close_window_and_reopen_bz,
    wait_for_cancel_confirmation, wait_for_collect_confirmation,
    is_terminal_purchase_failure_message,
    parse_bed_remaining_secs,
    parse_cookie_duration_secs,
    clear_auction_preview_slot, click_window_slot_carrying,
    count_empty_player_slots, find_dominant_inventory_item,
    build_cached_my_auctions_json, rebuild_cached_window_json,
    MANAGE_ORDERS_FALLBACK_SLOT, SELL_INVENTORY_NOW_FALLBACK_SLOT,
    MAX_CANCEL_RETRIES, MIN_FREE_SLOTS_FOR_BUY,
    WINDOW_CLOSE_DELAY_MS,
};
use azalea_protocol::packets::game::s_set_carried_item::ServerboundSetCarriedItem;
use azalea_protocol::packets::game::s_use_item::ServerboundUseItem;
use azalea_protocol::packets::game::s_interact::InteractionHand;
use crate::bot::client::{
    normalize_bazaar_order_text,
    extract_price_from_lore,
    CONFIRM_PURCHASE_RETRY_MS,
};


pub async fn handle_window_selling_inventory_bz(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        // Sell whole inventory instantly via /bz → "Sell Inventory Now"
        // → "Selling whole inventory" (slot 11).
        //
        // Flow (tracked by sell_inventory_step):
        //   Initial       — bazaar main page: find & click "Sell Inventory Now"
        //   ConfirmWindow — confirmation page: click slot 11 to confirm
        //
        // If inventory has no instasellable items, Hypixel sends
        // "You don't have anything to sell!" in chat and does not open
        // the confirmation window.
        let step = *state.bazaar.sell_inventory_step.read();
        if *state.last_window_id.read() != window_id {
            return;
        }

        if step == SellInventoryStep::Initial && window_title.contains("Bazaar") {
            // Bazaar page (main or category) — find "Sell Inventory Now"
            // button dynamically.
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            if *state.last_window_id.read() != window_id { return; }
            let slots = bot.menu().slots();
            let sell_inv_slot = find_slot_by_name(&slots, "Sell Inventory Now").unwrap_or(SELL_INVENTORY_NOW_FALLBACK_SLOT);
            info!("[SellInventoryBz] Bazaar window open — clicking 'Sell Inventory Now' at slot {}", sell_inv_slot);
            *state.bazaar.sell_inventory_step.write() = SellInventoryStep::ConfirmWindow;
            click_window_slot(bot, &state.last_window_id, window_id, sell_inv_slot as i16).await;
        } else if step == SellInventoryStep::ConfirmWindow {
            // Confirmation page — click slot 11 ("Selling whole inventory")
            info!("[SellInventoryBz] Confirmation window open — clicking slot 11 to sell");
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            if *state.last_window_id.read() != window_id { return; }
            click_window_slot(bot, &state.last_window_id, window_id, 11).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            info!("[SellInventoryBz] Done — closing window and going idle");
            send_raw_close(bot, window_id, &state.handlers);
            *state.bazaar.sell_inventory_step.write() = SellInventoryStep::Initial;
            *state.bot_state.write() = BotState::Idle;
        }
}

