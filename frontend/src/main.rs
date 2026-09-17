//! crypto-arb-frontend 主入口。

use crypto_arb_frontend::app::App;
use leptos::prelude::*;

fn main() {
    mount_to_body(App);
}
