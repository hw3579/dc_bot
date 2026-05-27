mod broker;
mod relay;
mod runtime_env;

use std::sync::Arc;
use std::time::Duration as StdDuration;

use broker::{relay_signal, BrokerConfig, BrokerKind};
use futures_util::StreamExt;
use relay::{
    build_snapshot, current_broker_config, current_nats_config,
    normalize_nats_server_address, persist_runtime_snapshot, queue_signal,
    signal_from_nats_payload, update_message, validate_signal, AppState, NatsFeedConfig,
    IbGatewayConfig, OptionSignalInput, RelayMessage, RelayReceipt,
    RuntimeSnapshot, SNAPSHOT_EVENT,
};
use tauri::{AppHandle, Emitter, Manager, State};

#[tauri::command]
fn bootstrap_state(state: State<AppState>) -> Result<RuntimeSnapshot, String> {
    build_snapshot(&state)
}

#[tauri::command]
fn save_broker_config(
    app: AppHandle,
    state: State<AppState>,
    config: BrokerConfig,
) -> Result<RuntimeSnapshot, String> {
    config.validate()?;

    {
        let mut stored_config = state
            .config
            .lock()
            .map_err(|_| String::from("无法更新 broker 配置"))?;
        *stored_config = config;
    }

    emit_snapshot(&app)
}

#[tauri::command]
fn save_ib_gateway_config(
    app: AppHandle,
    state: State<AppState>,
    config: IbGatewayConfig,
) -> Result<RuntimeSnapshot, String> {
    config.validate()?;

    {
        let mut stored_config = state
            .config
            .lock()
            .map_err(|_| String::from("无法更新 broker 配置"))?;

        stored_config.broker = BrokerKind::Ibkr;
        stored_config.host = config.host;
        stored_config.port = config.port;
        stored_config.client_id = config.client_id;
        stored_config.account = config.account;
        stored_config.default_exchange = config.default_exchange;
        stored_config.currency = config.currency;
        stored_config.dry_run = config.dry_run;
        stored_config.auto_forward = config.auto_forward;
    }

    emit_snapshot(&app)
}

#[tauri::command]
fn save_nats_feed_config(
    app: AppHandle,
    state: State<AppState>,
    config: NatsFeedConfig,
) -> Result<RuntimeSnapshot, String> {
    config.validate()?;

    {
        let mut stored_config = state
            .nats_config
            .lock()
            .map_err(|_| String::from("无法更新 NATS Feed 配置"))?;
        *stored_config = config;
    }

    let snapshot = emit_snapshot(&app)?;
    restart_app_nats_subscription(&app)?;
    Ok(snapshot)
}

#[tauri::command]
fn submit_option_signal(
    app: AppHandle,
    state: State<AppState>,
    signal: OptionSignalInput,
) -> Result<RelayMessage, String> {
    submit_signal(&app, &state, signal)
}

fn emit_snapshot(app: &AppHandle) -> Result<RuntimeSnapshot, String> {
    let snapshot = {
        let state = app.state::<AppState>();
        build_snapshot(&state)?
    };

    persist_runtime_snapshot(&snapshot)?;

    app.emit(SNAPSHOT_EVENT, snapshot.clone())
        .map_err(|error| error.to_string())?;

    Ok(snapshot)
}

fn mutate_message(
    app: &AppHandle,
    message_id: u64,
    mutate: impl FnOnce(&mut RelayMessage),
) -> Result<(), String> {
    let state = app.state::<AppState>();
    update_message(&state, message_id, mutate)?;

    emit_snapshot(app)?;
    Ok(())
}

fn submit_signal(
    app: &AppHandle,
    state: &AppState,
    signal: OptionSignalInput,
) -> Result<RelayMessage, String> {
    validate_signal(&signal)?;

    let (message, config) = queue_signal(state, signal.clone())?;
    emit_snapshot(app)?;

    if config.auto_forward {
        spawn_forward_task_for_app(app.clone(), message.id, signal, config);
    }

    Ok(message)
}

fn spawn_forward_task_for_app(
    app: AppHandle,
    message_id: u64,
    signal: OptionSignalInput,
    config: BrokerConfig,
) {
    tauri::async_runtime::spawn(async move {
        let _ = mutate_message(&app, message_id, |entry| {
            entry.status = relay::RelayStatus::Forwarding;
            entry.relay_notes = config.forwarding_note();
        });

        match relay_signal(&signal, &config).await {
            Ok(receipt) => {
                let notes = receipt.message.clone();
                let _ = mutate_message(&app, message_id, |entry| {
                    entry.status = relay::RelayStatus::Sent;
                    entry.relay_notes = notes;
                    entry.receipt = Some(receipt);
                });
            }
            Err(error) => {
                let failure_receipt = RelayReceipt {
                    broker: config.active_broker_id().to_string(),
                    order_id: None,
                    message: error.clone(),
                    simulated: false,
                };

                let _ = mutate_message(&app, message_id, |entry| {
                    entry.status = relay::RelayStatus::Failed;
                    entry.relay_notes = error;
                    entry.receipt = Some(failure_receipt);
                });
            }
        }
    });
}

fn restart_app_nats_subscription(app: &AppHandle) -> Result<(), String> {
    let (generation, config) = {
        let state = app.state::<AppState>();
        let generation = state.advance_nats_generation();
        let config = current_nats_config(&state)?;
        (generation, config)
    };

    if !config.auto_subscribe {
        return Ok(());
    }

    config.validate()?;

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        run_app_nats_subscription_loop(app_handle, generation).await;
    });

    Ok(())
}

async fn run_app_nats_subscription_loop(app: AppHandle, generation: u64) {
    loop {
        let config = {
            let state = app.state::<AppState>();

            if state.nats_generation() != generation {
                return;
            }

            match current_nats_config(&state) {
                Ok(config) if config.auto_subscribe => config,
                Ok(_) => return,
                Err(error) => {
                    eprintln!("failed to read NATS config: {error}");
                    return;
                }
            }
        };

        let server_address = normalize_nats_server_address(&config.server_address);
        let client = match async_nats::connect(server_address.clone()).await {
            Ok(client) => client,
            Err(error) => {
                eprintln!(
                    "failed to connect to NATS {} for subject {}: {error}",
                    server_address, config.subject
                );

                if !app_subscription_generation_is_current(&app, generation) {
                    return;
                }

                tokio::time::sleep(StdDuration::from_secs(3)).await;
                continue;
            }
        };

        let mut subscriber = match subscribe_nats(&client, &config).await {
            Ok(subscriber) => subscriber,
            Err(error) => {
                eprintln!("failed to subscribe to {}: {error}", config.subject);

                if !app_subscription_generation_is_current(&app, generation) {
                    return;
                }

                tokio::time::sleep(StdDuration::from_secs(3)).await;
                continue;
            }
        };

        eprintln!(
            "nats subscriber attached to {} via {}",
            config.subject, server_address
        );

        loop {
            tokio::select! {
                maybe_message = subscriber.next() => {
                    match maybe_message {
                        Some(message) => {
                            if let Err(error) = handle_nats_message_for_app(&app, message.payload.as_ref()) {
                                eprintln!("failed to handle NATS message: {error}");
                            }
                        }
                        None => break,
                    }
                }
                _ = tokio::time::sleep(StdDuration::from_secs(2)) => {
                    if !app_subscription_generation_is_current(&app, generation) {
                        return;
                    }
                }
            }
        }

        if !app_subscription_generation_is_current(&app, generation) {
            return;
        }

        eprintln!("nats subscription dropped, reconnecting...");
        tokio::time::sleep(StdDuration::from_secs(2)).await;
    }
}

fn app_subscription_generation_is_current(app: &AppHandle, generation: u64) -> bool {
    let state = app.state::<AppState>();
    state.nats_generation() == generation
}

fn handle_nats_message_for_app(app: &AppHandle, payload: &[u8]) -> Result<(), String> {
    let state = app.state::<AppState>();
    let config = current_broker_config(&state)?;
    let signal = signal_from_nats_payload(payload, config.default_quantity)?;
    let _ = submit_signal(app, &state, signal)?;
    Ok(())
}

async fn subscribe_nats(
    client: &async_nats::Client,
    config: &NatsFeedConfig,
) -> Result<async_nats::Subscriber, String> {
    if config.queue_group.trim().is_empty() {
        client
            .subscribe(config.subject.clone())
            .await
            .map_err(|error| error.to_string())
    } else {
        client
            .queue_subscribe(config.subject.clone(), config.queue_group.clone())
            .await
            .map_err(|error| error.to_string())
    }
}

fn persist_headless_snapshot(state: &AppState) -> Result<RuntimeSnapshot, String> {
    let snapshot = build_snapshot(state)?;
    persist_runtime_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn submit_signal_headless(
    state: &Arc<AppState>,
    signal: OptionSignalInput,
) -> Result<RelayMessage, String> {
    validate_signal(&signal)?;

    let (message, config) = queue_signal(state.as_ref(), signal.clone())?;
    persist_headless_snapshot(state.as_ref())?;

    println!(
        "queued signal #{} {} {} {}{}",
        message.id,
        message.signal.side.as_label(),
        message.signal.symbol,
        message.signal.strike,
        message.signal.option_type.as_suffix()
    );

    if config.auto_forward {
        spawn_forward_task_for_headless(Arc::clone(state), message.id, signal, config);
    }

    Ok(message)
}

fn spawn_forward_task_for_headless(
    state: Arc<AppState>,
    message_id: u64,
    signal: OptionSignalInput,
    config: BrokerConfig,
) {
    tokio::spawn(async move {
        let _ = update_message(state.as_ref(), message_id, |entry| {
            entry.status = relay::RelayStatus::Forwarding;
            entry.relay_notes = config.forwarding_note();
        })
        .and_then(|_| persist_headless_snapshot(state.as_ref()).map(|_| ())); 

        match relay_signal(&signal, &config).await {
            Ok(receipt) => {
                let notes = receipt.message.clone();
                let _ = update_message(state.as_ref(), message_id, |entry| {
                    entry.status = relay::RelayStatus::Sent;
                    entry.relay_notes = notes;
                    entry.receipt = Some(receipt);
                })
                .and_then(|_| persist_headless_snapshot(state.as_ref()).map(|_| ()));

                println!("forwarded signal #{} successfully", message_id);
            }
            Err(error) => {
                let failure_receipt = RelayReceipt {
                    broker: config.active_broker_id().to_string(),
                    order_id: None,
                    message: error.clone(),
                    simulated: false,
                };

                let _ = update_message(state.as_ref(), message_id, |entry| {
                    entry.status = relay::RelayStatus::Failed;
                    entry.relay_notes = error.clone();
                    entry.receipt = Some(failure_receipt);
                })
                .and_then(|_| persist_headless_snapshot(state.as_ref()).map(|_| ()));

                eprintln!("failed to forward signal #{}: {error}", message_id);
            }
        }
    });
}

async fn run_headless_subscription_loop(state: Arc<AppState>) {
    loop {
        let config = match current_nats_config(state.as_ref()) {
            Ok(config) if config.auto_subscribe => config,
            Ok(_) => return,
            Err(error) => {
                eprintln!("failed to read NATS config: {error}");
                return;
            }
        };

        let server_address = normalize_nats_server_address(&config.server_address);
        let client = match async_nats::connect(server_address.clone()).await {
            Ok(client) => client,
            Err(error) => {
                eprintln!(
                    "failed to connect to NATS {} for subject {}: {error}",
                    server_address, config.subject
                );
                tokio::time::sleep(StdDuration::from_secs(3)).await;
                continue;
            }
        };

        let mut subscriber = match subscribe_nats(&client, &config).await {
            Ok(subscriber) => subscriber,
            Err(error) => {
                eprintln!("failed to subscribe to {}: {error}", config.subject);
                tokio::time::sleep(StdDuration::from_secs(3)).await;
                continue;
            }
        };

        println!(
            "headless relay subscribed to {} via {}",
            config.subject, server_address
        );

        while let Some(message) = subscriber.next().await {
            let default_quantity = match current_broker_config(state.as_ref()) {
                Ok(config) => config.default_quantity,
                Err(error) => {
                    eprintln!("failed to read broker config for quantity: {error}");
                    continue;
                }
            };

            match signal_from_nats_payload(message.payload.as_ref(), default_quantity) {
                Ok(signal) => {
                    if let Err(error) = submit_signal_headless(&state, signal) {
                        eprintln!("failed to queue headless signal: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("failed to convert NATS payload into relay signal: {error}");
                }
            }
        }

        eprintln!("headless NATS subscription dropped, reconnecting...");
        tokio::time::sleep(StdDuration::from_secs(2)).await;
    }
}

async fn run_headless_inner() -> Result<(), String> {
    let state = Arc::new(AppState::load());
    let snapshot = persist_headless_snapshot(state.as_ref())?;

    println!(
        "headless relay started with subject {} -> {}",
        snapshot.nats_config.subject,
        snapshot.broker_config.active_target_summary()
    );

    if snapshot.nats_config.auto_subscribe {
        snapshot.nats_config.validate()?;
        let state_for_task = Arc::clone(&state);
        tokio::spawn(async move {
            run_headless_subscription_loop(state_for_task).await;
        });
    } else {
        eprintln!(
            "headless mode is idle because NATS auto-subscribe is disabled; edit the client config JSON or use the GUI to save autoSubscribe=true"
        );
    }

    tokio::signal::ctrl_c()
        .await
        .map_err(|error| format!("headless relay 等待退出信号失败: {error}"))?;

    println!("headless relay stopping");
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::load())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            emit_snapshot(app.handle())?;
            restart_app_nats_subscription(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap_state,
            save_broker_config,
            save_ib_gateway_config,
            save_nats_feed_config,
            submit_option_signal
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

pub fn run_headless() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime for headless relay");

    if let Err(error) = runtime.block_on(run_headless_inner()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
