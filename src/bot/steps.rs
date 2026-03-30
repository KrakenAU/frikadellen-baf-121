/// GUI workflow step enums — one enum per BotState flow.
/// Imported by their respective window handlers.


/// Which step of the auction creation flow the bot is in.
/// Matches TypeScript's setPrice/durationSet flags in sellHandler.ts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuctionStep {
    #[default]
    Initial,       // Just sent /ah, waiting for "Auction House"
    OpenManage,    // Clicked slot 15 in AH, waiting for "Manage Auctions"
    ClickCreate,   // Clicked "Create Auction" in Manage Auctions, waiting for "Create Auction"
    SelectBIN,     // Clicked slot 48 in "Create Auction", waiting for "Create BIN Auction"
    PriceSign,     // Clicked item + slot 31, sign expected (setPrice=false in TS)
    SetDuration,   // Price sign done; "Create BIN Auction" second visit → click slot 33
    DurationSign,  // "Auction Duration" opened + slot 16 clicked; sign expected for duration
    ConfirmSell,   // Duration sign done; "Create BIN Auction" third visit → click slot 29
    FinalConfirm,  // In "Confirm BIN Auction" → click slot 11
}



#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BazaarStep {
    #[default]
    Initial,
    SearchResults,
    SelectOrderType,
    SetAmount,
    SetPrice,
    Confirm,
}


/// Steps for the InstaSelling flow (separate from bazaar order placement).
/// BotState::InstaSelling uses this instead of reusing BazaarStep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstaSellStep {
    #[default]
    FindItem,       // /bz opened — searching for the item in results
    FindSellButton, // On item detail page — looking for "Sell Instantly"
    WaitConfirm,    // On confirmation/warning page — waiting for "Confirm" button
}


/// Steps for the SellingInventoryBz flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SellInventoryStep {
    #[default]
    Initial,       // Bazaar main page — clicking "Sell Inventory Now"
    ConfirmWindow, // Confirmation page — clicking slot 11 to sell
}


/// Sub-steps within the BuyingCookie state.
/// Matches TypeScript cookieHandler.ts buyCookie() flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CookieStep {
    #[default]
    Initial,         // Sent /bz booster cookie, waiting for Bazaar window
    ItemDetail,      // Clicked cookie item (slot 11), waiting for detail window
    BuyConfirm,      // Clicked Buy Instantly (slot 10), waiting for confirm window
    WaitingForCookie, // Clicked Confirm, waiting for cookie to appear in inventory
    ConsumingCookie, // Right-clicked cookie, waiting for cookie GUI window
}

