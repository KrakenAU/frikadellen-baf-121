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


pub async fn handle_window_cancelling_auction(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        // Cancel auction flow: /ah → Manage Auctions → find auction → Cancel Auction → Confirm
        if window_title.contains("Auction House") {
            info!("[CancelAuction] Auction House opened - navigating to Manage Auctions");
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            if let Some(i) = find_slot_by_name(&slots, "Manage Auctions")
                .or_else(|| find_slot_by_name(&slots, "My Auctions"))
            {
                info!("[CancelAuction] Clicking Manage/My Auctions at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            } else {
                info!("[CancelAuction] Manage/My Auctions not found by name, clicking slot 15");
                click_window_slot(bot, &state.last_window_id, window_id, 15).await;
            }
        } else if is_my_auctions_window_title(window_title) {
            info!("[CancelAuction] Manage Auctions opened - searching for target auction");
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            let target_name = state.auction.cancel_item_name.read().clone();
            let target_bid = *state.auction.cancel_starting_bid.read();
            let target_lower = target_name.to_lowercase();
            // Find the auction slot matching item_name + starting_bid
            let mut found = false;
            for (i, item) in slots.iter().enumerate() {
                if item.is_empty() { continue; }
                let display_name = match get_item_display_name_from_slot(item) {
                    Some(n) => n,
                    None => continue,
                };
                let clean = remove_mc_colors(&display_name).to_lowercase();
                if !clean.contains(&target_lower) { continue; }
                // Verify price matches for accurate identification
                let lore = get_item_lore_from_slot(item);
                let price = extract_price_from_lore(&lore);
                if let Some(p) = price {
                    if p != target_bid { continue; }
                }
                // Verify this is an active auction (not sold/expired)
                let combined_lower = lore.join("\n").to_lowercase();
                if combined_lower.contains("sold!")
                    || combined_lower.contains("expired")
                    || combined_lower.contains("ended")
                {
                    continue;
                }
                info!("[CancelAuction] Found matching auction '{}' at slot {} (price: {:?})", display_name, i, price);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                found = true;
                break;
            }
            if !found {
                info!("[CancelAuction] Target auction not found in Manage Auctions, closing window and going idle");
                send_raw_close(bot, window_id, &state.handlers);
                *state.bot_state.write() = BotState::Idle;
            }
        } else if window_title.contains("BIN Auction View") || window_title.contains("Auction View") {
            info!("[CancelAuction] Auction detail opened - looking for Cancel Auction button");
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            if let Some(i) = find_slot_by_name(&slots, "Cancel Auction") {
                info!("[CancelAuction] Clicking Cancel Auction at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            } else {
                info!("[CancelAuction] Cancel Auction button not found, closing window and going idle");
                send_raw_close(bot, window_id, &state.handlers);
                *state.bot_state.write() = BotState::Idle;
            }
            // Watchdog: go idle if no follow-up window in 2s
            let cancel_state_ref = state.bot_state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
                if *cancel_state_ref.read() == BotState::CancellingAuction {
                    info!("[CancelAuction] No follow-up window after 2s, going idle");
                    *cancel_state_ref.write() = BotState::Idle;
                }
            });
        } else if window_title.contains("Confirm") {
            info!("[CancelAuction] Confirm window opened - confirming cancellation");
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            // Click Confirm button — typically slot 11 or find by name
            if let Some(i) = find_slot_by_name(&slots, "Confirm") {
                info!("[CancelAuction] Clicking Confirm at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            } else {
                info!("[CancelAuction] Confirm not found by name, clicking slot 11");
                click_window_slot(bot, &state.last_window_id, window_id, 11).await;
            }
            info!("[CancelAuction] Auction cancellation confirmed, closing window and going idle");
            send_raw_close(bot, window_id, &state.handlers);
            let cancelled_name = state.auction.cancel_item_name.read().clone();
            let cancelled_bid = *state.auction.cancel_starting_bid.read();
            let _ = state.event_tx.send(BotEvent::AuctionCancelled {
                item_name: cancelled_name,
                starting_bid: cancelled_bid as u64,
            });
            *state.bot_state.write() = BotState::Idle;
        }
}

