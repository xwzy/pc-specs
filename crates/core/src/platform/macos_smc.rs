//! macOS 系统管理控制器（SMC）的只读访问：CPU / GPU 逐核温度、风扇转速、整机功率、
//! 电源适配器的电压和电流。
//!
//! 走 `AppleSMC` 服务的用户态接口，读取不需要管理员权限（写入才需要，这里一概不做，
//! 所以也不会去动风扇）。各机型的键不完全一样，读不到的键直接跳过，
//! 同一份代码在 Intel 和 Apple 芯片的 Mac 上都能用。

use crate::model::SensorReading;
use std::ffi::{c_char, c_void};
use std::mem;
use std::sync::OnceLock;

type MachPort = u32;
type KernReturn = i32;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main_port: MachPort, matching: *mut c_void) -> MachPort;
    fn IOServiceOpen(
        service: MachPort,
        owning_task: MachPort,
        kind: u32,
        connect: *mut MachPort,
    ) -> KernReturn;
    fn IOServiceClose(connect: MachPort) -> KernReturn;
    fn IOObjectRelease(object: MachPort) -> KernReturn;
    fn IOConnectCallStructMethod(
        connection: MachPort,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> KernReturn;
}

unsafe extern "C" {
    /// C 头文件里 `mach_task_self()` 宏展开出来的就是这个全局变量。
    static mach_task_self_: MachPort;
}

/// `AppleSMC` 用户态接口只有一个入口，具体做什么由结构体里的命令字决定。
const HANDLE_YPC_EVENT: u32 = 2;
const CMD_READ_KEY: u8 = 5;
const CMD_KEY_INFO: u8 = 9;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Version {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PowerLimit {
    version: u16,
    length: u16,
    cpu: u32,
    gpu: u32,
    memory: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyInfo {
    data_size: u32,
    data_type: u32,
    attributes: u8,
}

/// 与内核约定的请求 / 应答结构（共 80 字节），字段顺序和对齐不能改。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Message {
    key: u32,
    version: Version,
    power_limit: PowerLimit,
    key_info: KeyInfo,
    result: u8,
    status: u8,
    command: u8,
    data32: u32,
    bytes: [u8; 32],
}

const _: () = assert!(mem::size_of::<Message>() == 80);

fn fourcc(name: &str) -> u32 {
    name.bytes().fold(0, |acc, b| (acc << 8) | u32::from(b))
}

pub struct Smc {
    connection: MachPort,
}

impl Smc {
    pub fn open() -> Option<Self> {
        // SAFETY: `IOServiceMatching` 返回的字典交给 `IOServiceGetMatchingService` 接管；
        // 拿到的服务用完即释放，连接由 `Drop` 关闭。
        unsafe {
            let matching = IOServiceMatching(c"AppleSMC".as_ptr());
            if matching.is_null() {
                return None;
            }
            let service = IOServiceGetMatchingService(0, matching);
            if service == 0 {
                return None;
            }
            let mut connection = 0;
            let status = IOServiceOpen(service, mach_task_self_, 0, &mut connection);
            IOObjectRelease(service);
            (status == 0 && connection != 0).then_some(Self { connection })
        }
    }

    fn call(&self, input: &Message) -> Option<Message> {
        let mut output = Message::default();
        let mut size = mem::size_of::<Message>();
        // SAFETY: 输入输出都是 80 字节的 `repr(C)` 结构，长度如实传入。
        let status = unsafe {
            IOConnectCallStructMethod(
                self.connection,
                HANDLE_YPC_EVENT,
                (input as *const Message).cast(),
                mem::size_of::<Message>(),
                (&mut output as *mut Message).cast(),
                &mut size,
            )
        };
        (status == 0 && output.result == 0).then_some(output)
    }

    /// 读一个键，按它自己声明的类型换算成数字。键不存在、类型不认识都返回 `None`。
    pub fn number(&self, key: &str) -> Option<f32> {
        let mut request = Message {
            key: fourcc(key),
            command: CMD_KEY_INFO,
            ..Default::default()
        };
        let info = self.call(&request)?.key_info;
        request.key_info.data_size = info.data_size;
        request.command = CMD_READ_KEY;
        let reply = self.call(&request)?;
        let size = (info.data_size as usize).min(reply.bytes.len());
        decode(info.data_type, &reply.bytes[..size])
    }
}

impl Drop for Smc {
    fn drop(&mut self) {
        // SAFETY: 关闭自己打开的那个连接。
        unsafe {
            IOServiceClose(self.connection);
        }
    }
}

/// SMC 的数据类型：浮点是主机字节序（小端），整数和定点数是大端。
fn decode(data_type: u32, bytes: &[u8]) -> Option<f32> {
    let be16 = || bytes.get(..2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let value = match &data_type.to_be_bytes() {
        b"flt " => f32::from_le_bytes(bytes.get(..4)?.try_into().ok()?),
        b"ui8 " => f32::from(*bytes.first()?),
        b"si8 " => f32::from(*bytes.first()? as i8),
        b"ui16" => f32::from(be16()?),
        b"si16" => f32::from(be16()? as i16),
        b"ui32" => u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?) as f32,
        // 8.8 定点（温度、旧机型的电压电流）与 14.2 定点（Intel 机型的风扇转速）。
        b"sp78" => f32::from(be16()? as i16) / 256.0,
        b"fpe2" => f32::from(be16()?) / 4.0,
        _ => return None,
    };
    value.is_finite().then_some(value)
}

/// 整机功率、适配器输入的功率 / 电压 / 电流：(键, 名称, 种类, 单位, 合理范围)。
const ELECTRICAL: &[(&str, &str, &str, &str, f32)] = &[
    ("PSTR", "System Total", "power", "W", 1000.0),
    ("PDTR", "DC In", "power", "W", 1000.0),
    ("PPBR", "Battery", "power", "W", 1000.0),
    ("VD0R", "DC In", "voltage", "V", 100.0),
    ("ID0R", "DC In", "current", "A", 100.0),
];

/// 风扇的数量上限：再多就是读到了脏数据。
const MAX_FANS: u32 = 8;

/// 一代 Apple 芯片上 CPU / GPU 逐核温度传感器的 SMC 键。
///
/// 这些键没有公开文档，是社区对每一代芯片实测整理出来的，Stats、iStat Menus 读的也是这一批。
/// 为什么不用 sysinfo 通过 HID 给的温度：Apple 芯片上它看到的是 `PMU tdie`，那是电源管理芯片
/// 自己的温度，满载时比 CPU 核心低三十多度，不能当 CPU 温度。
/// 同一代里标准 / Pro / Max / Ultra 的核数不同，键不会全有，读不到的跳过即可。
struct ChipSensors {
    efficiency: &'static [&'static str],
    performance: &'static [&'static str],
    /// M5 起新增的「超级核」。
    super_cores: &'static [&'static str],
    gpu: &'static [&'static str],
}

const M1: ChipSensors = ChipSensors {
    efficiency: &["Tp09", "Tp0T"],
    performance: &[
        "Tp01", "Tp05", "Tp0D", "Tp0H", "Tp0L", "Tp0P", "Tp0X", "Tp0b",
    ],
    super_cores: &[],
    gpu: &["Tg05", "Tg0D", "Tg0L", "Tg0T"],
};

const M2: ChipSensors = ChipSensors {
    efficiency: &["Tp1h", "Tp1t", "Tp1p", "Tp1l"],
    performance: &[
        "Tp01", "Tp05", "Tp09", "Tp0D", "Tp0X", "Tp0b", "Tp0f", "Tp0j",
    ],
    super_cores: &[],
    gpu: &["Tg0f", "Tg0j"],
};

const M3: ChipSensors = ChipSensors {
    efficiency: &["Te05", "Te0L", "Te0P", "Te0S"],
    performance: &[
        "Tf04", "Tf09", "Tf0A", "Tf0B", "Tf0D", "Tf0E", "Tf44", "Tf49", "Tf4A", "Tf4B", "Tf4D",
        "Tf4E",
    ],
    super_cores: &[],
    gpu: &[
        "Tf14", "Tf18", "Tf19", "Tf1A", "Tf24", "Tf28", "Tf29", "Tf2A",
    ],
};

const M4: ChipSensors = ChipSensors {
    efficiency: &["Te05", "Te0S", "Te09", "Te0H"],
    performance: &[
        "Tp01", "Tp05", "Tp09", "Tp0D", "Tp0V", "Tp0Y", "Tp0b", "Tp0e",
    ],
    super_cores: &[],
    // 标准版和 Pro / Max 各有一对自己的键，另外六个是共用的。
    gpu: &[
        "Tg0G", "Tg0H", "Tg1U", "Tg1k", "Tg0K", "Tg0L", "Tg0d", "Tg0e", "Tg0j", "Tg0k",
    ],
};

const M5: ChipSensors = ChipSensors {
    efficiency: &[],
    performance: &[
        "Tp0O", "Tp0R", "Tp0U", "Tp0X", "Tp0a", "Tp0d", "Tp0g", "Tp0j", "Tp0m", "Tp0p", "Tp0u",
        "Tp0y",
    ],
    super_cores: &["Tp00", "Tp04", "Tp08", "Tp0C", "Tp0G", "Tp0K"],
    gpu: &[
        "Tg0U", "Tg0X", "Tg0d", "Tg0g", "Tg0j", "Tg1Y", "Tg1c", "Tg1g",
    ],
};

const M6: ChipSensors = ChipSensors {
    efficiency: &["Te07", "Te08", "Te09", "Te0c", "Te0h", "Te0i"],
    performance: &[
        "Tp0m", "Tp0L", "Tp0I", "Tp0d", "Tp0E", "Tp0G", "Tp0b", "Tp05",
    ],
    super_cores: &["Tp0j", "Tp0g", "Tp09", "Tp07"],
    gpu: &[
        "Tg1e", "Tg0d", "Tg0c", "Tg0b", "Tg1c", "Tg1d", "Tg07", "Tg1f", "Tg09", "Tg0a", "Tg1g",
        "Tg1h",
    ],
};

const A18_PRO: ChipSensors = ChipSensors {
    efficiency: &["Te05", "Te0S"],
    performance: &["Tp05", "Tp0D"],
    super_cores: &[],
    gpu: &["Tg05", "Tg0D", "Tg0L", "Tg0e"],
};

/// 按 CPU 型号字符串（`Apple M2 Max`）找对应那一代的键表。Intel 和认不出的新芯片返回 `None`。
fn layout_for(brand: &str) -> Option<&'static ChipSensors> {
    let model = brand.trim().strip_prefix("Apple ")?;
    if model.starts_with("A18") {
        return Some(&A18_PRO);
    }
    let generation: u32 = model
        .strip_prefix('M')?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()?;
    match generation {
        1 => Some(&M1),
        2 => Some(&M2),
        3 => Some(&M3),
        4 => Some(&M4),
        5 => Some(&M5),
        6 => Some(&M6),
        _ => None,
    }
}

/// 本机这一代芯片的键表，型号只查一次。
fn chip_layout() -> Option<&'static ChipSensors> {
    static LAYOUT: OnceLock<Option<&'static ChipSensors>> = OnceLock::new();
    *LAYOUT.get_or_init(|| {
        crate::util::sysctl::string("machdep.cpu.brand_string").and_then(|brand| layout_for(&brand))
    })
}

/// 逐核温度的合理范围：键表之外的 `T*` 键里混着不少不是温度的东西（负数、一百多度的常量），
/// 键表里的键理论上都干净，这里再兜一道，防止某个型号上同名键换了含义。
fn plausible_temperature(celsius: f32) -> bool {
    (10.0..=125.0).contains(&celsius)
}

/// 按键表读出这一代芯片的逐核温度。读不到的键跳过，名字按读到的顺序从 1 编号。
fn core_temperatures(
    layout: &ChipSensors,
    read: impl Fn(&str) -> Option<f32>,
) -> Vec<SensorReading> {
    let groups: [(&str, &[&str]); 4] = [
        ("CPU efficiency core", layout.efficiency),
        ("CPU performance core", layout.performance),
        ("CPU super core", layout.super_cores),
        ("GPU", layout.gpu),
    ];
    let mut out = Vec::new();
    for (name, keys) in groups {
        let readable = keys
            .iter()
            .filter_map(|key| read(key))
            .filter(|value| plausible_temperature(*value));
        for (index, value) in readable.enumerate() {
            out.push(SensorReading {
                source: "smc".to_string(),
                label: format!("{name} {}", index + 1),
                kind: "temperature".to_string(),
                value,
                unit: "C".to_string(),
            });
        }
    }
    out
}

/// CPU / GPU 逐核温度、风扇转速、功率、电压、电流。机器不开放 SMC（虚拟机、沙盒）时是空的。
pub fn readings() -> Vec<SensorReading> {
    let Some(smc) = Smc::open() else {
        return Vec::new();
    };
    let reading = |label: String, kind: &str, value: f32, unit: &str| SensorReading {
        source: "smc".to_string(),
        label,
        kind: kind.to_string(),
        value,
        unit: unit.to_string(),
    };
    let mut out = chip_layout().map_or_else(Vec::new, |layout| {
        core_temperatures(layout, |key| smc.number(key))
    });

    let fans = smc
        .number("FNum")
        .map_or(0, |n| (n.max(0.0) as u32).min(MAX_FANS));
    for index in 0..fans {
        if let Some(rpm) = smc
            .number(&format!("F{index}Ac"))
            .filter(|rpm| (0.0..30_000.0).contains(rpm))
        {
            out.push(reading(format!("Fan {}", index + 1), "fan", rpm, "RPM"));
        }
    }

    for &(key, label, kind, unit, limit) in ELECTRICAL {
        if let Some(value) = smc.number(key).filter(|v| v.abs() < limit) {
            // 功率和电流取绝对值：方向（充 / 放）由电池页负责，这里只看大小。
            out.push(reading(label.to_string(), kind, value.abs(), unit));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(name: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*name)
    }

    #[test]
    fn decodes_the_types_the_smc_uses() {
        assert_eq!(
            decode(kind(b"flt "), &2312.5f32.to_le_bytes()),
            Some(2312.5)
        );
        assert_eq!(decode(kind(b"ui8 "), &[2]), Some(2.0));
        assert_eq!(decode(kind(b"si8 "), &[0xff]), Some(-1.0));
        assert_eq!(decode(kind(b"ui16"), &[0x4f, 0x15]), Some(20_245.0));
        assert_eq!(decode(kind(b"si16"), &[0xff, 0xfe]), Some(-2.0));
        assert_eq!(decode(kind(b"sp78"), &[0x1e, 0x80]), Some(30.5));
        assert_eq!(decode(kind(b"fpe2"), &[0x0f, 0xa0]), Some(1000.0));
        assert_eq!(decode(kind(b"ui32"), &[0, 0, 0x09, 0x4e]), Some(2382.0));
        // 数据不够长、类型不认识，都不能 panic。
        assert_eq!(decode(kind(b"flt "), &[1, 2]), None);
        assert_eq!(decode(kind(b"hex_"), &[1]), None);
        assert_eq!(decode(kind(b"flt "), &f32::NAN.to_le_bytes()), None);
    }

    #[test]
    fn keys_are_packed_big_endian() {
        assert_eq!(fourcc("FNum"), 0x464e_756d);
    }

    #[test]
    fn picks_the_sensor_table_by_chip_generation() {
        let key = |brand: &str| layout_for(brand).map(|l| l.performance.first().copied());
        assert_eq!(key("Apple M1"), Some(Some("Tp01")));
        assert_eq!(key("Apple M2 Max"), Some(Some("Tp01")));
        assert_eq!(key("Apple M3 Pro"), Some(Some("Tf04")));
        assert_eq!(key("Apple M4 Max"), Some(Some("Tp01")));
        assert_eq!(key("Apple M5"), Some(Some("Tp0O")));
        assert_eq!(key("Apple M6 Ultra"), Some(Some("Tp0m")));
        assert_eq!(key("Apple A18 Pro"), Some(Some("Tp05")));
        // Intel 和还没收录的新一代都没有键表，由上层退回 HID 读数。
        assert!(layout_for("Intel(R) Core(TM) i9-9980HK CPU @ 2.40GHz").is_none());
        assert!(layout_for("Apple M9").is_none());
        assert!(layout_for("Apple").is_none());
        assert!(layout_for("").is_none());
    }

    #[test]
    fn every_sensor_key_is_a_valid_unique_fourcc() {
        for (name, layout) in [
            ("M1", &M1),
            ("M2", &M2),
            ("M3", &M3),
            ("M4", &M4),
            ("M5", &M5),
            ("M6", &M6),
            ("A18 Pro", &A18_PRO),
        ] {
            // 同一组里出现重复的键，多半是抄表时抄错了。
            for (group, keys) in [
                ("efficiency", layout.efficiency),
                ("performance", layout.performance),
                ("super", layout.super_cores),
                ("gpu", layout.gpu),
            ] {
                let mut seen = std::collections::HashSet::new();
                for key in keys {
                    assert!(
                        key.len() == 4 && key.is_ascii() && key.starts_with('T'),
                        "{name} {group}: bad key {key:?}"
                    );
                    assert!(seen.insert(*key), "{name} {group}: duplicate {key}");
                }
            }
            // CPU 的几组键互不重叠，否则同一个传感器会被算两遍。
            let cpu: Vec<_> = layout
                .efficiency
                .iter()
                .chain(layout.performance)
                .chain(layout.super_cores)
                .collect();
            let unique: std::collections::HashSet<_> = cpu.iter().collect();
            assert_eq!(cpu.len(), unique.len(), "{name}: CPU keys overlap");
        }
    }

    #[test]
    fn reads_only_plausible_core_temperatures_and_numbers_them() {
        // 假的 SMC：M2 Max 上的几个键，加一个读出垃圾值的和一个不存在的。
        let read = |key: &str| match key {
            "Tp1h" => Some(83.8),
            "Tp1t" => Some(81.3),
            "Tp1p" => None,
            "Tp1l" => Some(-103.3),
            "Tp01" => Some(95.7),
            "Tp05" => Some(89.2),
            "Tp0D" => Some(0.0),
            "Tg0f" => Some(76.5),
            "Tg0j" => Some(78.4),
            _ => None,
        };
        let readings = core_temperatures(&M2, read);
        let summary: Vec<(String, f32)> = readings
            .iter()
            .map(|r| (r.label.clone(), r.value))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("CPU efficiency core 1".to_string(), 83.8),
                ("CPU efficiency core 2".to_string(), 81.3),
                ("CPU performance core 1".to_string(), 95.7),
                ("CPU performance core 2".to_string(), 89.2),
                ("GPU 1".to_string(), 76.5),
                ("GPU 2".to_string(), 78.4),
            ]
        );
        assert!(
            readings
                .iter()
                .all(|r| r.source == "smc" && r.kind == "temperature" && r.unit == "C")
        );
    }

    #[test]
    fn a_machine_without_any_readable_key_yields_nothing() {
        assert!(core_temperatures(&M2, |_| None).is_empty());
    }
}
