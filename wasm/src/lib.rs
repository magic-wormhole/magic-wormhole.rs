use std::{error::Error, path::PathBuf, rc::Rc};

use futures_util::future::FutureExt;
use log::Level;
use magic_wormhole::{transfer, transit, Code, Wormhole};
use wasm_bindgen::prelude::*;

mod event;

#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

/// convert an error to a string for using it as a [JsValue]
fn stringify(e: impl Error) -> String {
    format!("error code: {}", e)
}

#[wasm_bindgen]
/// Configuration for the wormhole communication.
pub struct WormholeConfig {
    rendezvous_url: String,
    relay_url: String,
}

#[wasm_bindgen]
impl WormholeConfig {
    /// Create a new wormhole configuration.
    /// * `rendezvous_url` - A string with the websocket address for the rendezvous server
    /// * `relay_url` - A string with the websocket address for the relay server
    ///
    /// **Attention**: To use the wasm-library on an https page, please use a secure websocket url
    /// (i.e. `wss://...`)
    pub fn new(rendezvous_url: String, relay_url: String) -> WormholeConfig {
        wasm_logger::init(wasm_logger::Config::new(Level::Info));

        #[cfg(feature = "console_error_panic_hook")]
        console_error_panic_hook::set_once();

        WormholeConfig {
            rendezvous_url,
            relay_url,
        }
    }
}

#[wasm_bindgen]
/// Send a file via wormhole.
/// * `config` - The configuration object for the wormhole, created by [WormholeConfig::new]
/// * `file` - The Blob with the file to send
/// * `file_name` - The name of the file to send
/// * `cancel` - A promise that cancels the transfer, if it is resolved
/// * `event_handler` - A callback function that receives [event::Event] objects with updates about
/// the sending status and progress
pub async fn send_file(
    config: WormholeConfig,
    file: web_sys::Blob,
    file_name: String,
    cancel: js_sys::Promise,
    event_handler: js_sys::Function,
) -> Result<JsValue, JsValue> {
    let event_handler_wrap = Rc::new(Box::new(move |e: event::Event| {
        event_handler
            .call1(&JsValue::null(), &JsValue::from_serde(&e).unwrap())
            .expect("progress_handler call should succeed");
    }) as Box<dyn Fn(event::Event)>);

    let file_content = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await?;
    let array = js_sys::Uint8Array::new(&file_content);
    let len = array.byte_length() as u64;
    let data_to_send: Vec<u8> = array.to_vec();

    let (server_welcome, connector) = Wormhole::connect_without_code(
        transfer::APP_CONFIG.rendezvous_url(config.rendezvous_url.into()),
        2,
    )
    .await
    .map_err(stringify)?;

    event_handler_wrap(event::code(server_welcome.code.0));

    let ph = event_handler_wrap.clone();
    let wormhole = connector.await.map_err(stringify)?;
    transfer::send_file(
        wormhole,
        url::Url::parse(&config.relay_url).unwrap(),
        &mut &data_to_send[..],
        PathBuf::from(file_name),
        len,
        transit::Abilities::FORCE_RELAY,
        |_info, _address| {
            event_handler_wrap(event::connected());
        },
        move |cur, total| {
            ph(event::progress(cur, total));
        },
        wasm_bindgen_futures::JsFuture::from(cancel).map(|_x| ()),
    )
    .await
    .map_err(stringify)?;

    Ok("".into())
}

#[derive(serde::Serialize, serde::Deserialize)]
/// A structure of receiving a file
pub struct ReceiveResult {
    data: Vec<u8>,
    filename: String,
    filesize: u64,
}

#[wasm_bindgen]
/// Receive a file via wormhole.
/// * `config` - The configuration object for the wormhole, created by [WormholeConfig::new]
/// * `code` - The wormhole code (e.g. 15-foo-bar)
/// * `cancel` - A promise that cancels the transfer, if it is resolved
/// * `event_handler` - A callback function that receives [event::Event] objects with updates about
/// the receiving status and progress
///
/// The returned promise resolves with a [ReceiveResult], containing the file contents and file metadata
pub async fn receive_file(
    config: WormholeConfig,
    code: String,
    cancel: js_sys::Promise,
    progress_handler: js_sys::Function,
) -> Result<JsValue, JsValue> {
    let event_handler = Rc::new(Box::new(move |e: event::Event| {
        progress_handler
            .call1(&JsValue::null(), &JsValue::from_serde(&e).unwrap())
            .expect("progress_handler call should succeed");
    }) as Box<dyn Fn(event::Event)>);

    let (server_welcome, wormhole) = Wormhole::connect_with_code(
        transfer::APP_CONFIG.rendezvous_url(config.rendezvous_url.into()),
        Code(code),
    )
    .await
    .map_err(stringify)?;

    event_handler(event::server_welcome(
        server_welcome.welcome.unwrap_or_default(),
    ));

    let req = transfer::request_file(
        wormhole,
        url::Url::parse(&config.relay_url).unwrap(),
        transit::Abilities::FORCE_RELAY,
        wasm_bindgen_futures::JsFuture::from(cancel.clone()).map(|_x| ()),
    )
    .await
    .map_err(stringify)?
    .ok_or("")?;

    let filename = req.filename.to_str().unwrap_or_default().to_string();
    let filesize = req.filesize;
    event_handler(event::file_metadata(filename.clone(), filesize));

    let ph = event_handler.clone();
    let mut file: Vec<u8> = Vec::new();
    req.accept(
        |_info, _address| {
            event_handler(event::connected());
        },
        move |cur, total| {
            ph(event::progress(cur, total));
        },
        &mut file,
        wasm_bindgen_futures::JsFuture::from(cancel).map(|_x| ()),
    )
    .await
    .map_err(stringify)?;

    let result = ReceiveResult {
        data: file,
        filename,
        filesize,
    };
    Ok(JsValue::from_serde(&result).unwrap())
}
