from __future__ import annotations

import json
import math
import sys
from pathlib import Path


def _ensure_repo_venv_on_path() -> None:
    roots = []

    try:
        cwd = Path.cwd().resolve()
    except OSError:
        cwd = None

    if cwd is not None:
        roots.extend([cwd, *cwd.parents])

    if "__file__" in globals():
        module_path = Path(__file__).resolve()
        roots.extend([module_path.parent, *module_path.parents])

    seen = set()
    for root in roots:
        marker = str(root)
        if marker in seen:
            continue
        seen.add(marker)

        for candidate in root.glob('.venv/lib/python*/site-packages'):
            path_text = str(candidate)
            if path_text not in sys.path:
                sys.path.insert(0, path_text)


def _import_sdk():
    _ensure_repo_venv_on_path()

    try:
        import moomoo as sdk

        return sdk
    except ImportError:
        import futu as sdk

        return sdk


def _raise_sdk_error(prefix: str, ret: object, payload: object) -> None:
    raise RuntimeError(f"{prefix} failed: ret={ret}, payload={payload}")


def _option_type_for_signal(sdk, signal: dict[str, object]):
    option_type = str(signal["optionType"]).strip().upper()
    if option_type == "CALL":
        return sdk.OptionType.CALL
    if option_type == "PUT":
        return sdk.OptionType.PUT
    raise RuntimeError(f"unsupported optionType={option_type}")


def _trade_side_for_signal(sdk, signal: dict[str, object]):
    side = str(signal["side"]).strip().upper()
    if side == "BUY":
        return sdk.TrdSide.BUY
    if side == "SELL":
        return sdk.TrdSide.SELL
    raise RuntimeError(f"unsupported side={side}")


def _load_option_code(sdk, config: dict[str, object], signal: dict[str, object]) -> str:
    quote_ctx = sdk.OpenQuoteContext(host=config["host"], port=int(config["port"]))
    try:
        market = str(config["market"]).strip().upper()
        code = f"{market}.{str(signal['symbol']).strip().upper()}"
        expiry = str(signal["expiry"]).strip()
        option_type = _option_type_for_signal(sdk, signal)
        target_strike = float(signal["strike"])

        ret, data = quote_ctx.get_option_chain(
            code=code,
            start=expiry,
            end=expiry,
            option_type=option_type,
        )
        if ret != sdk.RET_OK:
            _raise_sdk_error("get_option_chain", ret, data)

        if data is None or len(data.index) == 0:
            raise RuntimeError(
                f"option chain returned no rows for {code} {expiry} {signal['optionType']}"
            )

        rows = data[
            data["strike_price"].apply(
                lambda strike_price: math.isclose(float(strike_price), target_strike, rel_tol=0.0, abs_tol=1e-6)
            )
        ]
        if len(rows.index) == 0:
            raise RuntimeError(
                f"option chain has no strike match for {code} {expiry} {signal['optionType']} {target_strike}"
            )

        return str(rows.iloc[0]["code"])
    finally:
        quote_ctx.close()


def place_option_order(config_json: str, signal_json: str) -> str:
    sdk = _import_sdk()
    config = json.loads(config_json)
    signal = json.loads(signal_json)

    option_code = _load_option_code(sdk, config, signal)
    trade_ctx = sdk.OpenSecTradeContext(
        filter_trdmarket=getattr(sdk.TrdMarket, str(config["market"]).strip().upper()),
        host=config["host"],
        port=int(config["port"]),
        security_firm=getattr(
            sdk.SecurityFirm,
            str(config["security_firm"]).strip().upper(),
        ),
    )

    try:
        trd_env_name = str(config["trd_env"]).strip().upper()
        trd_env = getattr(sdk.TrdEnv, trd_env_name)

        if trd_env_name == "REAL":
            password_md5 = config.get("trade_password_md5")
            password = config.get("trade_password")
            if password_md5:
                ret, data = trade_ctx.unlock_trade(password_md5=password_md5)
            elif password:
                ret, data = trade_ctx.unlock_trade(password=password)
            else:
                raise RuntimeError(
                    "MOOMOO_TRADE_PASSWORD or MOOMOO_TRADE_PASSWORD_MD5 is required for REAL trading"
                )

            if ret != sdk.RET_OK:
                _raise_sdk_error("unlock_trade", ret, data)

        limit_price = signal.get("limitPrice")
        if limit_price is None:
            order_type = getattr(sdk.OrderType, "MARKET", sdk.OrderType.NORMAL)
            price = 0.0
        else:
            order_type = sdk.OrderType.NORMAL
            price = float(limit_price)

        kwargs = {
            "price": price,
            "qty": float(signal["quantity"]),
            "code": option_code,
            "trd_side": _trade_side_for_signal(sdk, signal),
            "order_type": order_type,
            "trd_env": trd_env,
            "remark": str(signal.get("strategyTag") or "discord:auto"),
            "time_in_force": getattr(
                sdk.TimeInForce,
                str(config["time_in_force"]).strip().upper(),
            ),
            "fill_outside_rth": bool(config["fill_outside_rth"]),
            "session": getattr(
                sdk.Session,
                str(config["session"]).strip().upper(),
            ),
        }
        if int(config.get("acc_id") or 0) > 0:
            kwargs["acc_id"] = int(config["acc_id"])

        ret, data = trade_ctx.place_order(**kwargs)
        if ret != sdk.RET_OK:
            _raise_sdk_error("place_order", ret, data)

        order_id = None
        if data is not None and len(data.index) > 0 and "order_id" in data.columns:
            order_id = str(data.iloc[0]["order_id"])

        return json.dumps(
            {
                "orderId": order_id,
                "message": f"订单已提交到 Moomoo，option code = {option_code}",
                "simulated": False,
            }
        )
    finally:
        trade_ctx.close()