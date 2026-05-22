mod relay;

use relay::{
    relay_to_ib, validate_signal, AppState, IbGatewayConfig, OptionSignalInput, RelayMessage,
    RelayReceipt, RelayStats, RelayStatus, RuntimeSnapshot, SNAPSHOT_EVENT,
};
use tauri::{AppHandle, Emitter, Manager, State};

#[tauri::command]
fn bootstrap_state(state: State<AppState>) -> Result<RuntimeSnapshot, String> {
    build_snapshot(&state)
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
            .map_err(|_| String::from("无法更新 IB 配置"))?;
        *stored_config = config;
    }

    emit_snapshot(&app)
}

#[tauri::command]
fn submit_option_signal(
    app: AppHandle,
    state: State<AppState>,
    signal: OptionSignalInput,
) -> Result<RelayMessage, String> {
    validate_signal(&signal)?;

    let config = state
        .config
        .lock()
        .map_err(|_| String::from("无法读取 IB 配置"))?
        .clone();

    let message = RelayMessage {
        id: state.next_message_id(),
        received_at: relay::timestamp_now()?,
        signal: signal.clone(),
        status: RelayStatus::Queued,
        relay_notes: if config.auto_forward {
            String::from("信号已入队，准备立即转发到 IBKR")
        } else {
            String::from("信号已入队，等待手动开启 Auto Relay")
        },
        receipt: None,
    };

    {
        let mut messages = state
            .messages
            .lock()
            .map_err(|_| String::from("无法写入本地消息队列"))?;
        messages.insert(0, message.clone());
    }

    emit_snapshot(&app)?;

    if config.auto_forward {
        let app_handle = app.clone();
        let signal_for_task = signal.clone();
        let message_id = message.id;

        tauri::async_runtime::spawn(async move {
            let _ = mutate_message(&app_handle, message_id, |entry| {
                entry.status = RelayStatus::Forwarding;
                entry.relay_notes = if config.dry_run {
                    String::from("Dry-run 已接管，正在模拟 IBKR 下单路径")
                } else {
                    format!(
                        "正在连接 IB Gateway {}:{} 并提交订单",
                        config.host, config.port
                    )
                };
            });

            match relay_to_ib(&signal_for_task, &config).await {
                Ok(receipt) => {
                    let notes = receipt.message.clone();
                    let _ = mutate_message(&app_handle, message_id, |entry| {
                        entry.status = RelayStatus::Sent;
                        entry.relay_notes = notes;
                        entry.receipt = Some(receipt);
                    });
                }
                Err(error) => {
                    let failure_receipt = RelayReceipt {
                        broker: String::from("ibkr"),
                        order_id: None,
                        message: error.clone(),
                        simulated: false,
                    };

                    let _ = mutate_message(&app_handle, message_id, |entry| {
                        entry.status = RelayStatus::Failed;
                        entry.relay_notes = error;
                        entry.receipt = Some(failure_receipt);
                    });
                }
            }
        });
    }

    Ok(message)
}

fn build_snapshot(state: &AppState) -> Result<RuntimeSnapshot, String> {
    let broker_config = state
        .config
        .lock()
        .map_err(|_| String::from("无法读取应用状态"))?
        .clone();

    let messages = state
        .messages
        .lock()
        .map_err(|_| String::from("无法读取消息队列"))?
        .clone();

    let mut stats = RelayStats::default();
    stats.total = messages.len();

    for message in &messages {
        match message.status {
            RelayStatus::Queued => stats.queued += 1,
            RelayStatus::Forwarding => stats.forwarding += 1,
            RelayStatus::Sent => stats.sent += 1,
            RelayStatus::Failed => stats.failed += 1,
        }
    }

    Ok(RuntimeSnapshot {
        broker_config,
        messages,
        stats,
    })
}

fn emit_snapshot(app: &AppHandle) -> Result<RuntimeSnapshot, String> {
    let snapshot = {
        let state = app.state::<AppState>();
        build_snapshot(&state)?
    };

    app.emit(SNAPSHOT_EVENT, snapshot.clone())
        .map_err(|error| error.to_string())?;

    Ok(snapshot)
}

fn mutate_message(
    app: &AppHandle,
    message_id: u64,
    mutate: impl FnOnce(&mut RelayMessage),
) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut messages = state
            .messages
            .lock()
            .map_err(|_| String::from("无法更新消息状态"))?;

        let entry = messages
            .iter_mut()
            .find(|message| message.id == message_id)
            .ok_or_else(|| format!("找不到消息 {}", message_id))?;

        mutate(entry);
    }

    emit_snapshot(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            bootstrap_state,
            save_ib_gateway_config,
            submit_option_signal
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
