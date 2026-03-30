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


pub async fn handle_window_claiming_purchased(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        if window_title.contains("Auction House") {
            // Hardcoded slot 13 for "Your Bids" navigation — matches TypeScript clickWindow(bot, 13)
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            info!("[ClaimPurchased] Auction House opened - clicking slot 13 (Your Bids)");
            click_window_slot(bot, &state.last_window_id, window_id, 13).await;
        } else if window_title.contains("Your Bids") {
            info!("[ClaimPurchased] Your Bids opened - looking for Claim All or Sold item");
            // Wait for ContainerSetContent to arrive and populate slots
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            let mut found = false;
            // First look for Claim All by name (most reliable, matches TypeScript pattern)
            if let Some(i) = find_slot_by_name(&slots, "Claim All") {
                info!("[ClaimPurchased] Found Claim All at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                send_raw_close(bot, window_id, &state.handlers);
                *state.bot_state.write() = BotState::Idle;
                found = true;
            }
            if !found {
                // Look for purchased item with "Status: Sold!" in lore (TypeScript pattern)
                for (i, item) in slots.iter().enumerate() {
                    let lore = get_item_lore_from_slot(item);
                    let lore_lower = lore.join("\n").to_lowercase();
                    if lore_lower.contains("status:") && lore_lower.contains("sold") {
                        info!("[ClaimPurchased] Found purchased item with Sold status at slot {}", i);
                        click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                        // Stay in ClaimingPurchased — next window should be BIN Auction View
                        found = true;
                        break;
                    }
                }
                if !found {
                    info!("[ClaimPurchased] Nothing to claim, closing window and going idle");
                    send_raw_close(bot, window_id, &state.handlers);
                    *state.bot_state.write() = BotState::Idle;
                }
            }
        } else if window_title.contains("BIN Auction View") || window_title.contains("Auction View") {
            info!("[ClaimPurchased] Auction View opened - clicking slot 31 to collect");
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            click_window_slot(bot, &state.last_window_id, window_id, 31).await;
            send_raw_close(bot, window_id, &state.handlers);
            *state.bot_state.write() = BotState::Idle;
        }
}

pub async fn handle_window_claiming_sold(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        if window_title.contains("Auction House") {
            info!("[ClaimSold] Auction House opened - navigating to Manage Auctions (slot 15)");
            // Wait for ContainerSetContent to arrive and populate slots
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            // Prefer name-based match so a Hypixel UI shift is handled automatically;
            // fall back to the well-known slot 15 (same fixed slot the Selling flow uses).
            if let Some(i) = find_slot_by_name(&slots, "Manage Auctions")
                .or_else(|| find_slot_by_name(&slots, "My Auctions"))
            {
                info!("[ClaimSold] Clicking Manage/My Auctions at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            } else {
                info!("[ClaimSold] Manage/My Auctions not found by name, clicking slot 15");
                click_window_slot(bot, &state.last_window_id, window_id, 15).await;
            }
        } else if is_my_auctions_window_title(window_title) {
            info!("[ClaimSold] My/Manage Auctions opened - looking for claimable items");
            // Wait for ContainerSetContent to arrive and populate slots
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            // Look for Claim All first
            if let Some(i) = find_slot_by_name(&slots, "Claim All") {
                info!("[ClaimSold] Clicking Claim All at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                // Claim All finishes everything — close window and go idle
                send_raw_close(bot, window_id, &state.handlers);
                *state.bot_state.write() = BotState::Idle;
            } else {
                // Look for first claimable item
                let mut found = false;
                for (i, item) in slots.iter().enumerate() {
                    if is_claimable_auction_slot(item) {
                        info!("[ClaimSold] Clicking claimable item at slot {}", i);
                        click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                        // Stay in ClaimingSold — Hypixel re-opens Manage Auctions after the detail
                        found = true;
                        break;
                    }
                }
                if !found {
                    info!("[ClaimSold] Nothing to claim, closing window and going idle");
                    send_raw_close(bot, window_id, &state.handlers);
                    *state.bot_state.write() = BotState::Idle;
                }
            }
        } else if window_title.contains("BIN Auction View") || window_title.contains("Auction View") {
            info!("[ClaimSold] Auction detail opened - looking for Claim button");
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            // Prefer fixed slot 31 in auction detail; use name matching only as fallback.
            let slot_31_name = slots.get(31).and_then(get_item_display_name_from_slot).unwrap_or_default();
            let slot_31_lower = remove_mc_colors(&slot_31_name).to_lowercase();
            if slot_31_lower.contains("claim") {
                info!("[ClaimSold] Clicking preferred Claim slot 31");
                click_window_slot(bot, &state.last_window_id, window_id, 31).await;
            } else if let Some(i) = find_slot_by_name(&slots, "Claim") {
                info!("[ClaimSold] Slot 31 not claimable, falling back to Claim name match at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            } else {
                info!("[ClaimSold] Claim button not found, clicking slot 31 fallback");
                click_window_slot(bot, &state.last_window_id, window_id, 31).await;
            }
            // Spawn a short watchdog: if Hypixel doesn't re-open Manage Auctions within
            // 1.5 s, transition to Idle so the command queue can proceed.
            let claim_state_ref = state.bot_state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
                if *claim_state_ref.read() == BotState::ClaimingSold {
                    info!("[ClaimSold] No follow-up window after 1.5s, going idle");
                    *claim_state_ref.write() = BotState::Idle;
                }
            });
        }
}

