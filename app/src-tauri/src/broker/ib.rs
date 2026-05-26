use ibapi::orders::{order_builder, Action};
use ibapi::prelude::{Client, Contract, Currency, Exchange};

use crate::broker::BrokerConfig;
use crate::relay::{parse_expiry, validate_signal, OptionSignalInput, OptionType, OrderSide, RelayReceipt};

pub async fn relay(
    signal: &OptionSignalInput,
    config: &BrokerConfig,
) -> Result<RelayReceipt, String> {
    validate_signal(signal)?;

    let contract = build_contract(signal, config)?;
    let mut order = build_order(signal);

    if !config.account.trim().is_empty() {
        order.account = config.account.trim().to_string();
    }

    if config.dry_run {
        return Ok(RelayReceipt {
            broker: String::from("ibkr"),
            order_id: None,
            message: format!(
                "dry-run: 已生成 {} {} {} {}{} 合约与订单",
                signal.side.as_label(),
                signal.quantity,
                signal.symbol.trim().to_uppercase(),
                signal.strike,
                signal.option_type.as_suffix(),
            ),
            simulated: true,
        });
    }

    let connection_url = format!("{}:{}", config.host.trim(), config.port);
    let client = Client::connect(connection_url.as_str(), config.client_id)
        .await
        .map_err(|error| format!("连接 IB Gateway 失败: {error}"))?;

    let order_id = client.next_order_id();
    client
        .place_order(order_id, &contract, &order)
        .await
        .map_err(|error| format!("提交订单失败: {error}"))?;

    Ok(RelayReceipt {
        broker: String::from("ibkr"),
        order_id: Some(order_id.to_string()),
        message: format!("订单已提交到 {}，order id = {}", connection_url, order_id),
        simulated: false,
    })
}

fn build_contract(signal: &OptionSignalInput, config: &BrokerConfig) -> Result<Contract, String> {
    let (year, month, day) = parse_expiry(&signal.expiry)?;

    let mut contract = match signal.option_type {
        OptionType::Call => Contract::call(signal.symbol.trim().to_uppercase())
            .strike(signal.strike)
            .expires_on(year, month, day)
            .build(),
        OptionType::Put => Contract::put(signal.symbol.trim().to_uppercase())
            .strike(signal.strike)
            .expires_on(year, month, day)
            .build(),
    };

    contract.exchange = Exchange::from(config.default_exchange.trim());
    contract.currency = Currency::from(config.currency.trim());

    if contract.multiplier.is_empty() {
        contract.multiplier = String::from("100");
    }

    Ok(contract)
}

fn build_order(signal: &OptionSignalInput) -> ibapi::orders::Order {
    let action = match signal.side {
        OrderSide::Buy => Action::Buy,
        OrderSide::Sell => Action::Sell,
    };

    match signal.limit_price {
        Some(limit_price) => order_builder::limit_order(action, signal.quantity, limit_price),
        None => order_builder::market_order(action, signal.quantity),
    }
}