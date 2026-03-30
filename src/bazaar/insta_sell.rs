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


pub async fn handle_window_insta_selling(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        // Sell a dominant inventory item via /bz → Sell Instantly to free space.
        // Triggered by ManageOrders when inventory is full and one item type occupies
        // more than half the player inventory slots.
        //
        // Flow (tracked by insta_sell_step):
        //   FindItem       — bazaar search page: find item by name, click it
        //   FindSellButton — item detail page: find "Sell Instantly", click it
        //   WaitConfirm    — confirmation/warning page: wait ≤5 s, confirm
        //
        // After confirmation the bot opens /bz and returns to ManagingOrders so the
        // collect loop can retry now that there is inventory space.
        let item_name = match state.bazaar.insta_sell_item.read().clone() {
            Some(name) => name,
            None => {
                warn!("[InstaSell] No item name stored, closing window and going idle");
                send_raw_close(bot, window_id, &state.handlers);
                *state.bot_state.write() = BotState::Idle;
                return;
            }
        };

        // Abort if this window has already been superseded
        if *state.last_window_id.read() != window_id {
            return;
        }

        let step = *state.bazaar.insta_sell_step.read();
        info!("[InstaSell] Window: \"{}\" | step: {:?} | item: \"{}\"", window_title, step, item_name);

        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        if *state.last_window_id.read() != window_id { return; }

        if step == InstaSellStep::FindItem && window_title.contains("Bazaar") {
            // Search results: find the item by name and click it
            let poll_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(1500);
            let item_slot = loop {
                if *state.last_window_id.read() != window_id { return; }
                let slots = bot.menu().slots();
                if let Some(i) = find_slot_by_name(&slots, &item_name) {
                    break Some(i);
                }
                if tokio::time::Instant::now() >= poll_deadline { break None; }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            };
            match item_slot {
                Some(i) => {
                    if *state.last_window_id.read() != window_id { return; }
                    info!("[InstaSell] Found \"{}\" at slot {}, clicking", item_name, i);
                    *state.bazaar.insta_sell_step.write() = InstaSellStep::FindSellButton;
                    click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                }
                None => {
                    warn!("[InstaSell] Item \"{}\" not found in bazaar search, closing window and going idle", item_name);
                    send_raw_close(bot, window_id, &state.handlers);
                    *state.bazaar.insta_sell_item.write() = None;
                    *state.bazaar.insta_sell_step.write() = InstaSellStep::FindItem;
                    *state.bot_state.write() = BotState::Idle;
                }
            }
        } else if step == InstaSellStep::FindSellButton {
            // Item detail page: find "Sell Instantly" and click it
            let poll_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(1500);
            let sell_slot = loop {
                if *state.last_window_id.read() != window_id { return; }
                let slots = bot.menu().slots();
                if let Some(i) = find_slot_by_name(&slots, "Sell Instantly") {
                    break Some(i);
                }
                if tokio::time::Instant::now() >= poll_deadline { break None; }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            };
            match sell_slot {
                Some(i) => {
                    if *state.last_window_id.read() != window_id { return; }
                    info!("[InstaSell] Clicking \"Sell Instantly\" at slot {}", i);
                    *state.bazaar.insta_sell_step.write() = InstaSellStep::WaitConfirm;
                    click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                }
                None => {
                    warn!("[InstaSell] \"Sell Instantly\" not found, closing window and going idle");
                    send_raw_close(bot, window_id, &state.handlers);
                    *state.bazaar.insta_sell_item.write() = None;
                    *state.bazaar.insta_sell_step.write() = InstaSellStep::FindItem;
                    *state.bot_state.write() = BotState::Idle;
                }
            }
        } else if step == InstaSellStep::WaitConfirm {
            // Confirmation page (warning may be present for up to 5 seconds).
            // Wait up to 5 s for a "Confirm" button, then click it.
            info!("[InstaSell] Waiting up to 5s for confirm button...");
            let confirm_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
            let confirm_slot = loop {
                if *state.last_window_id.read() != window_id { return; }
                let slots = bot.menu().slots();
                if let Some(i) = find_slot_by_name(&slots, "Confirm") {
                    break Some(i);
                }
                if tokio::time::Instant::now() >= confirm_deadline { break None; }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            };
            if *state.last_window_id.read() != window_id { return; }
            match confirm_slot {
                Some(i) => {
                    info!("[InstaSell] Clicking Confirm at slot {}", i);
                    click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                }
                None => {
                    // Confirm button did not appear within 5 s — the sell may have already
                    // completed silently (no warning shown) or failed.  Log and continue so
                    // ManageOrders can retry; avoid clicking a random slot.
                    warn!("[InstaSell] Confirm button not found after 5s — sell may have completed or failed");
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            // Reset and return to ManagingOrders so collect can retry
            info!("[InstaSell] Complete — returning to ManageOrders");
            *state.bazaar.insta_sell_item.write() = None;
            *state.bazaar.insta_sell_step.write() = InstaSellStep::FindItem;
            state.inventory_full.store(false, Ordering::Relaxed);
            *state.bot_state.write() = BotState::ManagingOrders;
            send_chat_command(bot, "/bz");
        }
}

