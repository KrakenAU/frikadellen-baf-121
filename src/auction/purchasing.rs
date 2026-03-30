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


pub async fn handle_window_purchasing(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        if window_title.contains("BIN Auction View") || window_title.contains("Auction View") {
            // purchase_start_time is set in execute_command when
            // /viewauction is sent, so buy speed measures
            // command-send → coins-in-escrow.

            // Log the time from /viewauction to the spawned task
            // actually starting.  The delta between this and the
            // OpenScreen handler log shows the tokio::spawn overhead
            // (typically <1 ms unless the runtime is saturated).
            if let Some(t0) = *state.purchase_start_time.read() {
                info!(
                    "[Timing] /viewauction → interaction handler started: {:.1}ms",
                    t0.elapsed().as_secs_f64() * 1000.0
                );
            }

            // ---- Wait for slot 31 before clicking (ANTI-CHEAT GATE) ----
            // Do NOT click slot 31 or send skip-click until we know what
            // item the server placed there.  Clicking on a non-interactive
            // item (e.g. feather = loading placeholder) is an "impossible
            // action" that can trigger Hypixel anti-cheat.  The buy-click
            // and skip-click are sent AFTER gold_nugget is confirmed.
            //
            // The PacketAcceleratorPlugin fires slot_data_notify earlier
            // (during ECS apply_deferred rather than via Event::Packet),
            // which makes this loop exit ~20-70ms sooner on busy servers.
            // This is safe because the gold_nugget check still runs before
            // ANY click is sent — the accelerator just detects the server's
            // ContainerSetContent packet faster, not before it arrives.
            //
            // Wait for ContainerSetContent / ContainerSetSlot to populate
            // slot 31.  Uses a Notify that fires instantly when the packet
            // arrives instead of polling every 10ms.
            let slot_31_kind = {
                let poll_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(500);
                let mut kind;
                loop {
                    // Register the Notify listener BEFORE reading slots.
                    // notify_waiters() drops the notification when no task
                    // is waiting, so if ContainerSetContent fires between
                    // the slot read and the select! below, the notification
                    // would be lost and we'd stall for the full 500ms
                    // deadline.  Registering first guarantees we capture it.
                    let notified = state.slot_data_notify.notified();

                    let menu = bot.menu();
                    let slots = menu.slots();
                    kind = slots.get(31).map(|s| {
                        if s.is_empty() { "air".to_string() }
                        else { s.kind().to_string().to_lowercase() }
                    }).unwrap_or_else(|| "air".to_string());
                    if kind != "air" || tokio::time::Instant::now() >= poll_deadline {
                        break;
                    }
                    let remaining = poll_deadline - tokio::time::Instant::now();
                    tokio::select! {
                        _ = notified => {}
                        _ = tokio::time::sleep(remaining) => {}
                    }
                }
                kind
            };

            // Log when slot 31 data is ready — the delta between this
            // and the interaction handler start shows how long we waited
            // for ContainerSetContent / ContainerSetSlot.  When the
            // server sends both OpenScreen and ContainerSetContent in the
            // same TCP segment this is ~0 ms; otherwise it is one extra
            // ECS cycle (~3.3 ms at 300 fps).
            if let Some(t0) = *state.purchase_start_time.read() {
                info!(
                    "[Timing] /viewauction → slot 31 ready ({}): {:.1}ms",
                    slot_31_kind,
                    t0.elapsed().as_secs_f64() * 1000.0
                );
            }

            if slot_31_kind.contains("bed") {
                // Bed = auction is still in grace period.
                // No buy-click or skip-click was sent (we waited for
                // confirmation first).  The bed-spam loop below
                // repeatedly clicks slot 31 until the grace period ends
                // and the item becomes purchasable (gold_nugget appears).
                // Signal the 5-second GUI watchdog to leave this window open.
                state.bed_timing_active.store(true, Ordering::Relaxed);

                const BED_FREEMONEY_CLICK_INTERVAL_MS: u64 = 20;
                const BED_WAIT_POLL_MS: u64 = 20;
                const MAX_FAILED_CLICKS: usize = 5;
                let click_interval_ms = if state.freemoney {
                    BED_FREEMONEY_CLICK_INTERVAL_MS
                } else {
                    state.bed_spam_click_delay.max(1)
                };

                if state.freemoney {
                    // Freemoney mode: use COFL purchaseAt timing for bed auctions.
                    // Start clicking bed_pre_click_ms (default 30ms) before the deadline.
                    // If purchaseAt is not available, fall through to immediate bed spam.
                    let pre_click_lead_ms = state.bed_pre_click_ms;
                    // Convert the raw epoch-ms timestamp to a remaining-ms delta.
                    let remaining_ms_from_purchase_at = state.pending_purchase_at_ms.read()
                        .and_then(|purchase_at_ms| {
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .ok()
                                .map(|d| d.as_millis() as i64)?;
                            let diff = purchase_at_ms - now_ms;
                            if diff <= 0 { None } else { Some(diff as u64) }
                        });

                    if let Some(remaining_ms) = remaining_ms_from_purchase_at {
                        let wait_ms = remaining_ms.saturating_sub(pre_click_lead_ms);
                        if wait_ms > 0 {
                            info!("[AH] Bed timing (freemoney): purchaseAt in {}ms — waiting {}ms, then clicking at {}ms intervals (lead: {}ms)",
                                remaining_ms, wait_ms, click_interval_ms, pre_click_lead_ms);
                            let wait_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(wait_ms);
                            loop {
                                if tokio::time::Instant::now() >= wait_deadline {
                                    break;
                                }
                                let kind_now = {
                                    let menu = bot.menu();
                                    let slots = menu.slots();
                                    slots.get(31).map(|s| {
                                        if s.is_empty() { "air".to_string() }
                                        else { s.kind().to_string().to_lowercase() }
                                    }).unwrap_or_else(|| "air".to_string())
                                };
                                if !kind_now.contains("bed") {
                                    break;
                                }
                                tokio::time::sleep(tokio::time::Duration::from_millis(BED_WAIT_POLL_MS)).await;
                            }
                            info!("[AH] Bed timing (freemoney): entering rapid-click phase ({}ms interval)", click_interval_ms);
                        } else {
                            info!("[AH] Bed timing (freemoney): purchaseAt imminent — starting clicks at {}ms interval", click_interval_ms);
                        }
                    } else {
                        info!("[AH] Bed timing (freemoney): no purchaseAt — starting immediate bed spam at {}ms interval", click_interval_ms);
                    }
                } else {
                    // Default mode: simple immediate bed spam at bed_spam_click_delay.
                    info!("[AH] Bed detected in slot 31 — starting bed spam at {}ms interval", click_interval_ms);
                }

                let bed_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(70);
                let mut failed_clicks: usize = 0;
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_millis(click_interval_ms)).await;

                    if tokio::time::Instant::now() >= bed_deadline {
                        warn!("[AH] Bed timing: grace period did not end — giving up");
                        state.bed_timing_active.store(false, Ordering::Relaxed);
                        send_raw_close(bot, window_id, &state.handlers);
                        *state.bot_state.write() = BotState::Idle;
                        return;
                    }

                    let current_kind = {
                        let menu = bot.menu();
                        let slots = menu.slots();
                        slots.get(31).map(|s| {
                            if s.is_empty() { "air".to_string() }
                            else { s.kind().to_string().to_lowercase() }
                        }).unwrap_or_else(|| "air".to_string())
                    };

                    if current_kind == "air" || current_kind.contains("air") {
                        info!("[AH] Bed timing: window closed");
                        state.bed_timing_active.store(false, Ordering::Relaxed);
                        send_raw_close(bot, window_id, &state.handlers);
                        *state.bot_state.write() = BotState::Idle;
                        return;
                    } else if current_kind.contains("gold_nugget") {
                        info!("[AH] Bed timing: gold_nugget appeared, clicking slot 31");
                        state.bed_timing_active.store(false, Ordering::Relaxed);
                        if *state.last_window_id.read() == window_id {
                            send_raw_click(bot, window_id, 31);
                        }
                        break;
                    } else if current_kind.contains("potato") {
                        info!("[AH] Bed timing: potato detected — auction not purchasable, closing");
                        state.bed_timing_active.store(false, Ordering::Relaxed);
                        send_raw_close(bot, window_id, &state.handlers);
                        *state.bot_state.write() = BotState::Idle;
                        return;
                    } else if current_kind.contains("bed") {
                        if state.freemoney {
                            debug!("[AH] Bed timing: grace period active, pre-clicking slot 31");
                            if *state.last_window_id.read() == window_id {
                                send_raw_click(bot, window_id, 31);
                            }
                        } else {
                            debug!("[AH] Bed timing: grace period active, waiting for gold_nugget");
                        }
                    } else {
                        failed_clicks += 1;
                        debug!("[AH] Bed timing: slot 31 = {} (failed {}/{})", current_kind, failed_clicks, MAX_FAILED_CLICKS);
                        if failed_clicks >= MAX_FAILED_CLICKS {
                            warn!("[AH] Bed timing: stopped after {} unexpected slot states", failed_clicks);
                            state.bed_timing_active.store(false, Ordering::Relaxed);
                            send_raw_close(bot, window_id, &state.handlers);
                            *state.bot_state.write() = BotState::Idle;
                            return;
                        }
                    }
                }
            } else if slot_31_kind.contains("gold_nugget") {
                // ---- Buyable auction (SAFE: gold_nugget confirmed) ----
                // Now that we know slot 31 is gold_nugget (the buy button),
                // send the buy-click.  Sending AFTER confirmation avoids
                // clicking non-interactive items like feather (loading
                // placeholder) which is an "impossible action" that can
                // trigger Hypixel anti-cheat.
                //
                // Anti-cheat safety: This click only happens AFTER the
                // server has sent both OpenScreen AND ContainerSetContent
                // with gold_nugget in slot 31.  The PacketAcceleratorPlugin
                // reduces how long we wait to notice these packets, but the
                // click still occurs after the server has prepared the
                // window — which is exactly what a fast human player does.
                if let Some(t0) = *state.purchase_start_time.read() {
                    info!(
                        "[Timing] /viewauction → buy click sent: {:.1}ms",
                        t0.elapsed().as_secs_f64() * 1000.0
                    );
                }
                if state.fastbuy {
                    info!("[AH] gold_nugget confirmed — sending buy + skip (fastbuy)");
                } else {
                    info!("[AH] gold_nugget confirmed — sending buy click");
                }
                // Use raw connection to bypass ECS queue for the buy click.
                send_raw_click(bot, window_id, 31);

                // Skip-click: only when fastbuy is explicitly enabled,
                // pre-click slot 11 on the predicted Confirm Purchase
                // window in the same TCP burst as the buy-click.
                // When fastbuy is off the Confirm Purchase handler will
                // click confirm reactively when the window opens.
                if state.fastbuy {
                    // Redundant second buy click to guard against packet
                    // loss — only needed when we also send the skip-click,
                    // because a lost buy-click + queued confirm-click on a
                    // window that never opens is an impossible action.
                    send_raw_click(bot, window_id, 31);

                    let next_wid = if window_id == 255 { 1u8 } else { window_id + 1 };
                    info!("[AH] Fastbuy: pre-clicking slot 11 on predicted window {} (same burst)", next_wid);
                    state.skip_click_sent.store(true, Ordering::Relaxed);
                    // Use raw connection for the skip-click too.
                    send_raw_click(bot, next_wid, 11);
                }
            } else if !slot_31_kind.contains("air") {
                // ---- Non-buyable auction ----
                // Slot 31 is a non-purchasable item placed by the server:
                // feather = auction not available / cannot be purchased,
                // potato = already bought by someone else,
                // poisonous_potato = can't afford,
                // stained_glass_pane = edge case.
                // No clicks were sent (we waited for confirmation first),
                // so just close the window cleanly.
                // Use write lock for atomic check-and-set to prevent a
                // double-close race with the terminal-failure chat handler.
                let should_close = {
                    let mut bs = state.bot_state.write();
                    if *bs == BotState::Purchasing {
                        *bs = BotState::Idle;
                        true
                    } else {
                        false
                    }
                };
                if should_close {
                    warn!("[AH] Slot 31 = {} — auction not purchasable, closing window", slot_31_kind);
                    *state.purchase_start_time.write() = None;
                    *state.pending_purchase_at_ms.write() = None;
                    send_raw_close(bot, window_id, &state.handlers);
                }
            }
            // air = slot 31 never populated within the poll deadline.
            // The terminal-failure chat handler or 5s GUI watchdog will
            // clean up.
        } else if window_title.contains("Confirm Purchase") {
            // Log time from /viewauction to Confirm Purchase window.
            // This is the buy-click → server-processes → OpenScreen RTT.
            if let Some(t0) = *state.purchase_start_time.read() {
                info!(
                    "[Timing] /viewauction → Confirm Purchase opened: {:.1}ms",
                    t0.elapsed().as_secs_f64() * 1000.0
                );
            }
            // If a skip-click was already sent for this window, don't fire a
            // redundant reactive click — the pre-click packet should already be
            // queued on the server for the same tick.
            let skip_was_sent = state.skip_click_sent.swap(false, Ordering::Relaxed);
            if skip_was_sent {
                info!("[AH] Skip-click was sent — skipping reactive confirm click");
            } else {
                // No skip-click — click confirm (slot 11) immediately via raw connection.
                send_raw_click(bot, window_id, 11);
            }

            // When skip-click was sent, the server already has the confirm
            // click queued from the same TCP burst as the buy-click.  Wait
            // just long enough for the server to process it (2 ticks =
            // 100 ms) before retrying.  The previous 300 ms value was
            // overly conservative and caused a redundant retry click on
            // low-latency connections where the purchase completes in
            // ~50 ms but the window lingers for ~300 ms.
            //
            // Without skip-click the initial wait is shorter because the
            // click was just sent above and we want to retry quickly if it
            // was lost.
            let initial_wait_ms = if skip_was_sent { 100u64 } else { CONFIRM_PURCHASE_RETRY_MS };
            tokio::time::sleep(tokio::time::Duration::from_millis(initial_wait_ms)).await;

            // Safety retry loop: if the window is still open (pre-click failed,
            // click was lost, or the server needs more time), keep retrying.
            while state.handlers.current_window_title()
                .as_deref()
                .map(|t| t.contains("Confirm Purchase"))
                .unwrap_or(false)
            {
                send_raw_click(bot, window_id, 11);
                tokio::time::sleep(tokio::time::Duration::from_millis(CONFIRM_PURCHASE_RETRY_MS)).await;
            }

            send_raw_close(bot, window_id, &state.handlers);
            *state.bot_state.write() = BotState::Idle;
        }
}

