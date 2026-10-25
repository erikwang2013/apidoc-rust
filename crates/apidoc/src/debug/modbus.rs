//! Modbus TCP（feature `debug-industrial`，tokio-modbus）。
//!
//! 只做单目标、单请求：读线圈/离散输入/保持寄存器/输入寄存器，单线圈/单寄存器写。
//! 地址与数量在连接前校验，非法请求不触达设备；整个操作受 `DebugConfig.timeout_ms`。

use super::guard;
use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio_modbus::client::{tcp, Reader, Writer};
use tokio_modbus::Slave;

const DEFAULT_PORT: u16 = 502;

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let (host, port) = parse_target(&req.target)?;
    let addr = *guard::check_host(cfg, &host, port)
        .await?
        .first()
        .ok_or("target resolved to no addresses")?;
    let unit_id = opts_u16(req, "unit_id").unwrap_or(1);
    let operation = req.opts.get("operation").map(String::as_str).unwrap_or("read_holding");
    let address = opts_u16(req, "address").ok_or("modbus `address` is required")?;
    let quantity = opts_u16(req, "quantity").unwrap_or(1);

    // 先校验再连接：非法报文不触达设备。
    match operation {
        "read_coils" | "read_discrete" => validate_range(address, quantity, 2000)?,
        "read_holding" | "read_input" => validate_range(address, quantity, 125)?,
        "write_coil" | "write_register" => {}
        other => {
            return Err(format!(
                "unsupported modbus operation `{other}`; use read_coils, read_discrete, \
                 read_holding, read_input, write_coil or write_register"
            ))
        }
    }
    let register_value = if operation == "write_register" {
        Some(opts_u16(req, "value").ok_or("modbus `value` is required for write_register")?)
    } else {
        None
    };
    let coil_value = if operation == "write_coil" {
        Some(parse_bool(req.opts.get("value").map(String::as_str).unwrap_or("1"))?)
    } else {
        None
    };

    let unit_id = u8::try_from(unit_id)
        .map_err(|_| "modbus unit_id must fit u8 (0-255)".to_string())?;
    let timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    let exchange = async {
        let mut ctx = tcp::connect_slave(addr, Slave(unit_id))
            .await
            .map_err(|e| format!("modbus connect failed: {e}"))?;
        let values: Value = match operation {
            "read_coils" => json!(flat(ctx.read_coils(address, quantity).await)?),
            "read_discrete" => json!(flat(ctx.read_discrete_inputs(address, quantity).await)?),
            "read_holding" => json!(flat(ctx.read_holding_registers(address, quantity).await)?),
            "read_input" => json!(flat(ctx.read_input_registers(address, quantity).await)?),
            "write_coil" => {
                flat(ctx.write_single_coil(address, coil_value.unwrap_or(false)).await)?;
                json!([])
            }
            "write_register" => {
                flat(ctx.write_single_register(address, register_value.unwrap_or(0)).await)?;
                json!([])
            }
            _ => unreachable!("operation validated above"),
        };
        Ok::<Value, String>(values)
    };
    let values = tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| format!("modbus request timed out after {}ms", cfg.timeout_ms))??;

    let body = serde_json::to_vec(&json!({ "values": values }))
        .map_err(|e| format!("encode response: {e}"))?;
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers: BTreeMap::from([
            ("x-peer".into(), addr.to_string()),
            ("x-unit-id".into(), unit_id.to_string()),
        ]),
        body: Payload::from_bytes(&body),
        frames: Vec::new(),
    })
}

fn parse_target(target: &str) -> Result<(String, u16), String> {
    let with_scheme = if target.contains("://") {
        target.to_string()
    } else {
        format!("modbus://{target}")
    };
    let url = reqwest::Url::parse(&with_scheme).map_err(|e| format!("invalid target `{target}`: {e}"))?;
    match url.scheme() {
        "modbus" | "modbus-tcp" | "tcp" => {}
        other => return Err(format!("unsupported modbus target scheme `{other}`")),
    }
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    Ok((host, url.port().unwrap_or(DEFAULT_PORT)))
}

fn validate_range(address: u16, quantity: u16, max: u16) -> Result<(), String> {
    if quantity == 0 || quantity > max {
        return Err(format!("modbus quantity must be 1..={max}, got {quantity}"));
    }
    if u32::from(address) + u32::from(quantity) > 0x1_0000 {
        return Err(format!(
            "modbus address range {address}+{quantity} exceeds the 16-bit address space"
        ));
    }
    Ok(())
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "on" => Ok(true),
        "0" | "false" | "off" => Ok(false),
        other => Err(format!("invalid coil value `{other}`; use 0/1 or true/false")),
    }
}

/// 两层 Result：传输/协议错误 + 设备异常码，统一成字符串。
fn flat<T>(result: tokio_modbus::Result<T>) -> Result<T, String> {
    result
        .map_err(|e| format!("modbus request failed: {e}"))
        .and_then(|inner| inner.map_err(|e| format!("modbus exception: {e:?}")))
}

fn opts_u16(req: &DebugRequest, key: &str) -> Option<u16> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_and_quantity_bounds() {
        assert!(validate_range(0, 1, 125).is_ok());
        assert!(validate_range(0, 125, 125).is_ok());
        assert!(validate_range(0, 126, 125).is_err());
        assert!(validate_range(0, 0, 2000).is_err());
        assert!(validate_range(65_535, 2, 125).is_err());
    }

    #[test]
    fn coil_values_accept_common_spellings() {
        assert!(parse_bool("1").unwrap());
        assert!(parse_bool("TRUE").unwrap());
        assert!(!parse_bool("off").unwrap());
        assert!(parse_bool("2").is_err());
    }

    #[test]
    fn target_defaults_to_502() {
        assert_eq!(parse_target("host").unwrap(), ("host".into(), 502));
        assert_eq!(parse_target("tcp://host:1502").unwrap(), ("host".into(), 1502));
        assert!(parse_target("udp://host:1").is_err());
    }
}
