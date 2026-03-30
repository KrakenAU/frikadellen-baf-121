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


pub async fn handle_window_managing_orders(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        // Bazaar order management. In startup mode (cancel_open=true) all existing orders
        // are cancelled after collecting filled ones. In collect-only mode (cancel_open=false,
        // used when triggered by BazaarOrderFilled or periodic checks) only filled orders
        // are collected and open orders are left untouched.
        // Flow: Bazaar window → find & click "Manage Orders" → iterate orders → handle each.

        // Check the internal deadline at every window event.  If exceeded, close
        // this window and return to Idle so the command queue isn't blocked.
        if check_manage_orders_deadline(bot, state, window_id) {
            return;
        }

        let cancel_open = state.bazaar.manage_orders_cancel_open.load(Ordering::Relaxed);
        if window_title.contains("Bazaar") && !is_bazaar_orders_window_title(window_title) {
            // Bazaar page (main or category) — find "Manage Orders" button
            // dynamically.  Hypixel may rearrange slots across updates, so we
            // search by name instead of relying on a hardcoded slot index.
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            if *state.last_window_id.read() != window_id { return; }
            let slots = bot.menu().slots();
            let manage_slot = find_slot_by_name(&slots, "Manage Orders").unwrap_or(MANAGE_ORDERS_FALLBACK_SLOT);
            info!("[ManageOrders] Bazaar window open, clicking Manage Orders (slot {})", manage_slot);
            click_window_slot(bot, &state.last_window_id, window_id, manage_slot as i16).await;
        } else if is_bazaar_orders_window_title(window_title) {
            // ── Process ONE order per ManageOrders cycle ──
            // On Hypixel, clicking a filled order slot directly collects items/coins.
            // If the order is open (nothing to collect), "Order options" opens instead.
            // We process only one order then go Idle so bazaar flips aren't blocked.
            let mode_str = if cancel_open { "cancel+collect" } else { "collect-only" };
            info!("[ManageOrders] Processing orders ({}) — single order per cycle", mode_str);
            let persistent_processed = &state.bazaar.manage_orders_processed;

            // Wait for ContainerSetContent to populate the window
            tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
            if check_manage_orders_deadline(bot, state, window_id) { return; }
            if *state.last_window_id.read() != window_id { return; }

            let slots = bot.menu().slots();

            // ── Scan all order slots and pick the highest-priority one ──
            // Priority (collect-only mode): claimable/filled orders first,
            //   SELL before BUY within each tier.
            // Priority (cancel mode): SELL first, then BUY (all orders).
            //
            // Tuple: (slot, name, identity, order_key, claimable, filled_amount)
            type OrderEntry = (usize, String, Option<(bool, String)>, String, bool, Option<u64>);
            let mut sell_orders: Vec<OrderEntry> = Vec::new();
            let mut buy_orders: Vec<OrderEntry> = Vec::new();

            for (i, item) in slots.iter().enumerate() {
                if let Some(name) = get_item_display_name_from_slot(item) {
                    let lore = get_item_lore_from_slot(item);
                    let name_key = normalize_bazaar_order_text(&name);
                    if persistent_processed.read().contains(&name_key) {
                        continue;
                    }
                    let identity = parse_bazaar_order_identity(&name, &lore);
                    if !should_treat_as_bazaar_order_slot(&name, identity.as_ref()) {
                        continue;
                    }
                    let is_buy = identity.as_ref()
                        .map(|(b, _)| *b)
                        .unwrap_or_else(|| is_buy_bazaar_order_name(&name));
                    let claimable = is_order_claimable_from_lore(&lore);
                    let filled_amount = parse_filled_amount_from_lore(&lore).map(|(f, _)| f);
                    let order_key = format!("{}::{}", i, name_key);
                    if is_buy {
                        buy_orders.push((i, name, identity, order_key, claimable, filled_amount));
                    } else {
                        sell_orders.push((i, name, identity, order_key, claimable, filled_amount));
                    }
                }
            }

            let total_orders = sell_orders.len() + buy_orders.len();

            // Emit a reconciliation snapshot so the tracker can remove
            // stale entries that no longer exist in-game.
            {
                let mut ingame_orders = Vec::with_capacity(total_orders);
                for (_, name, identity, _, _, _) in sell_orders.iter().chain(buy_orders.iter()) {
                    let is_buy = identity.as_ref()
                        .map(|(b, _)| *b)
                        .unwrap_or_else(|| is_buy_bazaar_order_name(name));
                    let clean = clean_order_item_name(name, identity);
                    ingame_orders.push((clean, is_buy));
                }
                let _ = state.event_tx.send(BotEvent::BazaarOrdersSnapshot { ingame_orders });
            }

            // In collect-only mode, ONLY click claimable orders.  Open
            // (unfilled) orders have nothing to collect and clicking them
            // opens "Order options" which wastes a cycle and can leave the
            // bot stuck for the entire periodic-check interval.
            // However, stale orders that exceed the cancel-due-to-age threshold
            // must still be selected so they can be cancelled.
            let cancel_mins = state.bazaar.cancel_minutes_per_million;
            let mut inv_full = state.inventory_full.load(Ordering::Relaxed);
            // When the flag is set, double-check the actual inventory.
            // The flag can become stale after a manual instasell or
            // delayed Hypixel reminder ("stashed away").  If there are
            // enough free slots to hold a stack, clear the flag and let
            // BUY orders through.
            if inv_full {
                let empty = count_empty_player_slots(&bot);
                if empty >= MIN_FREE_SLOTS_FOR_BUY as usize {
                    info!("[ManageOrders] inventory_full flag was set but {} empty slots found — clearing flag", empty);
                    state.inventory_full.store(false, Ordering::Relaxed);
                    inv_full = false;
                }
            }
            let chosen_order: Option<OrderEntry> = if !cancel_open {
                // Always try claimable sell orders first (yield coins, no
                // inventory space needed).
                sell_orders.iter().find(|&(_, _, _, _, claimable, _)| *claimable).cloned()
                    // Claimable buy orders — skip entirely when inventory is
                    // full so we don't waste a ManageOrders cycle opening /bz,
                    // navigating to the order, and then aborting.
                    .or_else(|| {
                        if inv_full { None } else {
                            buy_orders.iter().find(|&(_, _, _, _, claimable, _)| *claimable).cloned()
                        }
                    })
                    .or_else(|| sell_orders.iter().find(|&(_, _, identity, _, claimable, _)| {
                        !claimable && should_cancel_open_order_due_to_age(identity.clone(), cancel_mins)
                    }).cloned())
                    .or_else(|| buy_orders.iter().find(|&(_, _, identity, _, claimable, _)| {
                        !claimable && should_cancel_open_order_due_to_age(identity.clone(), cancel_mins)
                    }).cloned())
            } else {
                // cancel mode: sell first, then buy (original behaviour)
                sell_orders.into_iter().next()
                    .or_else(|| buy_orders.into_iter().next())
            };

            match chosen_order {
                None => {
                    // No orders to process — done.
                    info!("[ManageOrders] No actionable orders, closing window");
                    send_raw_close(bot, window_id, &state.handlers);
                    *state.bazaar.manage_orders_deadline.write() = None;
                    state.bazaar.at_limit.store(false, Ordering::Relaxed);
                    *state.bot_state.write() = BotState::Idle;
                }
                Some((i, order_name, order_identity, _processed_key, _claimable, order_filled_amount)) => {
                    let order_is_buy = order_identity.as_ref()
                        .map(|(b, _)| *b)
                        .unwrap_or_else(|| is_buy_bazaar_order_name(&order_name));

                    // Skip BUY orders when inventory is full (no room for items).
                    // Exception: cancel_open mode still clicks to cancel.
                    // Re-check actual inventory to catch space freed by manual
                    // instasell or other actions since the flag was set.
                    if order_is_buy && !cancel_open {
                        let still_full = state.inventory_full.load(Ordering::Relaxed);
                        if still_full {
                            let empty = count_empty_player_slots(&bot);
                            if empty >= MIN_FREE_SLOTS_FOR_BUY as usize {
                                info!("[ManageOrders] inventory_full flag stale — {} empty slots, proceeding with BUY order \"{}\"", empty, order_name);
                                state.inventory_full.store(false, Ordering::Relaxed);
                            } else {
                                warn!("[ManageOrders] Inventory full ({} empty slots) — skipping BUY order \"{}\"", empty, order_name);
                                log_pending_claim(&order_name);
                                persistent_processed.write().insert(normalize_bazaar_order_text(&order_name));
                                send_raw_close(bot, window_id, &state.handlers);
                                *state.bazaar.manage_orders_deadline.write() = None;
                                *state.bot_state.write() = BotState::Idle;
                                return;
                            }
                        }
                    }

                    // Store context for the Order options handler (Branch C)
                    *state.bazaar.managing_order_context.write() = Some((order_is_buy, order_name.clone(), order_identity.clone(), order_filled_amount));

                    info!("[ManageOrders] Clicking order at slot {}: \"{}\"", i, order_name);
                    click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;

                    // Wait up to 5 seconds for Hypixel's response.
                    // • If "Order options" opens → order is open, Branch C handles it.
                    // • If the window doesn't change → the order was collected directly
                    //   by clicking the slot (Hypixel collects filled orders on click).
                    // • If a new "Bazaar Orders" window opens (same list, new ID) →
                    //   the order was collected and Hypixel refreshed the list.
                    let click_deadline = tokio::time::Instant::now()
                        + tokio::time::Duration::from_secs(5);
                    let mut order_options_opened = false;
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                        if *state.last_window_id.read() != window_id {
                            // A new window opened — check its title to distinguish
                            // "Order options" (order is open) from a refreshed
                            // "Bazaar Orders" list (order was collected on click).
                            if let Some(new_title) = state.handlers.current_window_title() {
                                if new_title.to_lowercase().contains("order options") {
                                    order_options_opened = true;
                                }
                                // else: Hypixel re-opened the orders list after
                                // collecting — treat as successful direct collection.
                            } else {
                                // No title available — assume Order options for safety.
                                order_options_opened = true;
                            }
                            break;
                        }
                        if state.inventory_full.load(Ordering::Relaxed) && order_is_buy {
                            break;
                        }
                        if tokio::time::Instant::now() >= click_deadline {
                            break;
                        }
                    }

                    if !order_options_opened {
                        // No "Order options" window → order was collected by clicking.
                        // Emit the collected event.
                        info!("[ManageOrders] Order \"{}\" collected (no Order options opened)", order_name);
                        let _ = state.event_tx.send(BotEvent::BazaarOrderCollected {
                            item_name: clean_order_item_name(&order_name, &order_identity),
                            is_buy_order: order_is_buy,
                            // Direct collection = fully filled. Use parsed filled
                            // amount from lore; falls back to tracker amount in handler.
                            claimed_amount: order_filled_amount,
                        });
                        persistent_processed.write().insert(normalize_bazaar_order_text(&order_name));
                        state.bazaar.at_limit.store(false, Ordering::Relaxed);
                    }
                    // else: "Order options" opened — Branch C handler takes over.
                    // It will handle collect/cancel/skip and then go Idle.

                    // Close this window and go Idle (one order per cycle).
                    if *state.last_window_id.read() == window_id {
                        send_raw_close(bot, window_id, &state.handlers);
                    }
                    if !order_options_opened {
                        // Only transition to Idle when the order was collected
                        // directly (no Order options window).  When Order options
                        // opened, the Order options handler (Branch C) takes over
                        // and is responsible for closing, clearing the deadline,
                        // and transitioning to Idle.  Setting Idle here races with
                        // the Branch C handler and can cause it to miss the
                        // ManagingOrders state, leaving the window stuck open.
                        *state.bazaar.manage_orders_deadline.write() = None;
                        *state.bot_state.write() = BotState::Idle;
                    }

                    // If there are more orders to process, immediately
                    // re-queue ManageOrders so we don't wait for the
                    // periodic timer (up to 60s) between each order.
                    // Only re-queue when the order was collected directly
                    // (order_options_opened == false).  When Order options
                    // opened, the Order options handler takes over and will
                    // go Idle after it finishes — we must not re-queue here
                    // because that handler is still active.
                    if total_orders > 1 && !order_options_opened {
                        if let Some(queue) = state.command_queue.read().as_ref() {
                            if !queue.has_manage_orders() {
                                info!("[ManageOrders] {} more order(s) remain — re-queuing ManageOrders", total_orders - 1);
                                queue.enqueue(
                                    crate::types::CommandType::ManageOrders { cancel_open },
                                    crate::types::CommandPriority::Normal,
                                    false,
                                );
                            }
                        }
                    }
                }
            }
        } else if window_title.to_lowercase().contains("order options") {
            // Hypixel opened "Order options" after clicking an order in the list.
            // This means the order is OPEN (not filled — filled orders collect on click).
            // After handling ONE order, close and go Idle (one order per cycle).

            let order_ctx = state.bazaar.managing_order_context.read().clone();
            let (order_name, order_identity, order_filled_amount) = match &order_ctx {
                Some((_is_buy, name, identity, filled)) => (name.clone(), identity.clone(), *filled),
                None => (String::new(), None, None),
            };
            let order_identity_for_clean = order_ctx.as_ref().and_then(|(_, _, id, _)| id.clone());

            let name_key = normalize_bazaar_order_text(&order_name);
            if !name_key.is_empty() && state.bazaar.manage_orders_processed.read().contains(&name_key) {
                debug!("[ManageOrders] Order \"{}\" already processed — closing", order_name);
                send_raw_close(bot, window_id, &state.handlers);
                *state.bazaar.manage_orders_deadline.write() = None;
                *state.bot_state.write() = BotState::Idle;
            } else {
            // Determine cancel_due_to_age BEFORE deciding whether to skip.
            // This allows stale orders to be cancelled even in collect-only mode.
            let cancel_due_to_age = !cancel_open
                && should_cancel_open_order_due_to_age(order_identity, state.bazaar.cancel_minutes_per_million);

            // Check cancel retry limit early — if exceeded, close immediately.
            let cancel_fail_key = normalize_bazaar_order_text(&order_name);
            let prior_failures = *state.bazaar.order_cancel_failures.read().get(&cancel_fail_key).unwrap_or(&0);
            let cancel_exceeded = prior_failures >= MAX_CANCEL_RETRIES;

            if cancel_exceeded && (cancel_open || cancel_due_to_age) {
                warn!(
                    "[ManageOrders] Order \"{}\" exceeded {} cancel attempts — closing GUI and giving up",
                    order_name, MAX_CANCEL_RETRIES
                );
                if !name_key.is_empty() {
                    state.bazaar.manage_orders_processed.write().insert(name_key);
                }
                if *state.last_window_id.read() == window_id {
                    send_raw_close(bot, window_id, &state.handlers);
                }
                *state.bazaar.manage_orders_deadline.write() = None;
                state.bazaar.at_limit.store(false, Ordering::Relaxed);
                *state.bot_state.write() = BotState::Idle;
            } else if !cancel_open && !cancel_due_to_age {
                // Collect-only mode and order is NOT stale.
                // "Order options" opened, so this order is not 100% filled (those
                // collect on click).  It may be PARTIALLY filled (has a Collect
                // button) or completely open (no Collect button).
                // Look for a Collect button first — if found, collect the partial
                // fill before closing.
                let probe_deadline =
                    tokio::time::Instant::now() + tokio::time::Duration::from_secs(2);
                let mut collect_slot_probe: Option<usize> = None;
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    if *state.last_window_id.read() != window_id { break; }
                    let slots_probe = bot.menu().slots();
                    collect_slot_probe = find_slot_by_name(&slots_probe, "Collect")
                        .or_else(|| find_slot_by_name(&slots_probe, "Claim"))
                        .or_else(|| find_slot_by_lore_contains(&slots_probe, "click to collect"))
                        .or_else(|| find_slot_by_lore_contains(&slots_probe, "claim your"));
                    if collect_slot_probe.is_some() { break; }
                    // Also check for Cancel — if present but no Collect, order is
                    // truly open with nothing to collect.
                    let has_cancel = find_slot_by_name(&slots_probe, "Cancel").is_some()
                        || find_slot_by_lore_contains(&slots_probe, "click to cancel").is_some();
                    if has_cancel { break; }
                    if tokio::time::Instant::now() >= probe_deadline { break; }
                }

                if let Some(cs) = collect_slot_probe {
                    // Partially filled order — collect before closing.
                    info!("[ManageOrders] Partially filled order \"{}\" — clicking Collect at slot {} (collect-only)", order_name, cs);
                    if *state.last_window_id.read() == window_id {
                        click_window_slot(bot, &state.last_window_id, window_id, cs as i16).await;
                        if wait_for_collect_confirmation(bot, &state.last_window_id, window_id).await {
                            if let Some((ctx_is_buy, _, _, _)) = order_ctx.as_ref() {
                                let _ = state.event_tx.send(BotEvent::BazaarOrderCollected {
                                    item_name: clean_order_item_name(&order_name, &order_identity_for_clean),
                                    is_buy_order: *ctx_is_buy,
                                    // Partial fill — use filled amount parsed from
                                    // the Manage Orders lore before we clicked.
                                    claimed_amount: order_filled_amount,
                                });
                            }
                        } else {
                            warn!("[ManageOrders] Collect click for partially filled \"{}\" was not confirmed", order_name);
                        }
                    }
                } else {
                    debug!("[ManageOrders] Order \"{}\" is open — skipping in collect-only mode", order_name);
                }

                if !name_key.is_empty() {
                    state.bazaar.manage_orders_processed.write().insert(name_key);
                }
                if *state.last_window_id.read() == window_id {
                    send_raw_close(bot, window_id, &state.handlers);
                }
                *state.bazaar.manage_orders_deadline.write() = None;
                *state.bot_state.write() = BotState::Idle;

                // Re-queue so remaining orders are processed without
                // waiting for the full periodic-check interval.
                if collect_slot_probe.is_some() {
                    if let Some(queue) = state.command_queue.read().as_ref() {
                        if !queue.has_manage_orders() {
                            info!("[ManageOrders] Re-queuing ManageOrders after partial collect in Order options");
                            queue.enqueue(
                                crate::types::CommandType::ManageOrders { cancel_open },
                                crate::types::CommandPriority::Normal,
                                false,
                            );
                        }
                    }
                }
            } else {
            // cancel_open mode OR cancel_due_to_age: look for Cancel/Collect buttons.
            info!(
                "[ManageOrders] Order options window opened ({}) — looking for Cancel/Collect buttons",
                if cancel_open { "startup cancel mode" } else { "cancel due to age" }
            );

            let action_deadline =
                tokio::time::Instant::now() + tokio::time::Duration::from_secs(3);
            let mut cancel_slot: Option<usize> = None;
            let mut collect_slot: Option<usize> = None;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                if *state.last_window_id.read() != window_id {
                    break;
                }
                let slots2 = bot.menu().slots();
                collect_slot = find_slot_by_name(&slots2, "Collect")
                    .or_else(|| find_slot_by_name(&slots2, "Claim"))
                    .or_else(|| find_slot_by_lore_contains(&slots2, "click to collect"))
                    .or_else(|| find_slot_by_lore_contains(&slots2, "claim your"));
                cancel_slot = find_slot_by_name(&slots2, "Cancel")
                    .or_else(|| find_slot_by_lore_contains(&slots2, "click to cancel"))
                    .or_else(|| find_slot_by_lore_contains(&slots2, "cancel order"));
                if collect_slot.is_some() || cancel_slot.is_some() {
                    break;
                }
                if tokio::time::Instant::now() >= action_deadline {
                    let slot_names: Vec<String> = slots2
                        .iter()
                        .enumerate()
                        .filter_map(|(idx, item)| {
                            get_item_display_name_from_slot(item).map(|n| format!("{}={}", idx, n))
                        })
                        .collect();
                    warn!(
                        "[ManageOrders] No Collect/Cancel button found in Order options — visible slots: [{}]",
                        slot_names.join(", ")
                    );
                    break;
                }
            }

            if cancel_due_to_age && cancel_slot.is_some() {
                info!(
                    "[ManageOrders] Open order \"{}\" exceeds cancel threshold ({}m/M) — will cancel (Order options)",
                    order_name, state.bazaar.cancel_minutes_per_million
                );
            }

            if let Some(cs) = collect_slot {
                if *state.last_window_id.read() == window_id {
                    info!("[ManageOrders] Clicking Collect at slot {} in Order options", cs);
                    click_window_slot(bot, &state.last_window_id, window_id, cs as i16).await;
                    if wait_for_collect_confirmation(bot, &state.last_window_id, window_id).await {
                        if let Some((ctx_is_buy, _, _, _)) = order_ctx.as_ref() {
                            let _ = state.event_tx.send(BotEvent::BazaarOrderCollected {
                                item_name: clean_order_item_name(&order_name, &order_identity_for_clean),
                                is_buy_order: *ctx_is_buy,
                                // Partial or full fill from Order Options — use
                                // filled amount parsed from the Manage Orders lore.
                                claimed_amount: order_filled_amount,
                            });
                        }
                    } else {
                        warn!("[ManageOrders] Collect click for \"{}\" was not confirmed in Order options", order_name);
                    }
                    if (cancel_open || cancel_due_to_age) && !cancel_exceeded {
                        if let Some(cancel_after) = find_slot_by_name(&bot.menu().slots(), "Cancel") {
                            if *state.last_window_id.read() == window_id {
                                info!("[ManageOrders] Clicking Cancel at slot {} after collecting in Order options", cancel_after);
                                click_window_slot(bot, &state.last_window_id, window_id, cancel_after as i16).await;
                                if wait_for_cancel_confirmation(bot, &state.last_window_id, window_id).await {
                                    *state.manage_orders_cancelled.write() += 1;
                                    state.bazaar.order_cancel_failures.write().remove(&cancel_fail_key);
                                    if let Some((ctx_is_buy, _, _, _)) = order_ctx.as_ref() {
                                        let _ = state.event_tx.send(BotEvent::BazaarOrderCancelled {
                                            item_name: clean_order_item_name(&order_name, &order_identity_for_clean),
                                            is_buy_order: *ctx_is_buy,
                                            already_collected: true,
                                        });
                                    }
                                } else {
                                    *state.bazaar.order_cancel_failures.write().entry(cancel_fail_key.clone()).or_insert(0) += 1;
                                    warn!("[ManageOrders] Cancel click for \"{}\" was not confirmed in Order options (attempt {})", order_name, prior_failures + 1);
                                }
                            }
                        }
                    }
                }
            } else if let Some(cs) = cancel_slot {
                if (cancel_open || cancel_due_to_age) && !cancel_exceeded {
                    if *state.last_window_id.read() == window_id {
                        info!("[ManageOrders] Clicking Cancel at slot {} in Order options", cs);
                        click_window_slot(bot, &state.last_window_id, window_id, cs as i16).await;
                        if wait_for_cancel_confirmation(bot, &state.last_window_id, window_id).await {
                            *state.manage_orders_cancelled.write() += 1;
                            state.bazaar.order_cancel_failures.write().remove(&cancel_fail_key);
                            if let Some((ctx_is_buy, _, _, _)) = order_ctx.as_ref() {
                                let _ = state.event_tx.send(BotEvent::BazaarOrderCancelled {
                                    item_name: clean_order_item_name(&order_name, &order_identity_for_clean),
                                    is_buy_order: *ctx_is_buy,
                                    already_collected: false,
                                });
                            }
                        } else {
                            *state.bazaar.order_cancel_failures.write().entry(cancel_fail_key.clone()).or_insert(0) += 1;
                            warn!("[ManageOrders] Cancel click for \"{}\" was not confirmed in Order options (attempt {})", order_name, prior_failures + 1);
                        }
                    }
                } else if !cancel_open && !cancel_due_to_age {
                    debug!("[ManageOrders] Skipping open order \"{}\" in Order options (collect-only mode)", order_name);
                }
            } else {
                warn!("[ManageOrders] No actionable button in Order options for \"{}\"", order_name);
            }

            // Mark as processed, close, and go Idle (one order per cycle)
            if !name_key.is_empty() {
                state.bazaar.manage_orders_processed.write().insert(name_key);
            }
            if *state.last_window_id.read() == window_id {
                send_raw_close(bot, window_id, &state.handlers);
            }
            *state.bazaar.manage_orders_deadline.write() = None;
            state.bazaar.at_limit.store(false, Ordering::Relaxed);
            *state.bot_state.write() = BotState::Idle;

            // Re-queue so remaining orders are processed promptly.
            if let Some(queue) = state.command_queue.read().as_ref() {
                if !queue.has_manage_orders() {
                    info!("[ManageOrders] Re-queuing ManageOrders after Order options (cancel/collect)");
                    queue.enqueue(
                        crate::types::CommandType::ManageOrders { cancel_open },
                        crate::types::CommandPriority::Normal,
                        false,
                    );
                }
            }
            }
            }
        } else {
            // Unexpected window title while in ManagingOrders state.
            // Re-navigate to /bz to get back on track.
            warn!(
                "[ManageOrders] Unexpected window \"{}\" — closing and re-opening /bz",
                window_title
            );
            close_window_and_reopen_bz(bot, state, window_id).await;
        }
}

