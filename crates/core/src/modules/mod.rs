//! 各系统信息模块。每个 `collect()` 都是无状态函数、彼此独立，可以并行执行。

pub mod battery;
pub mod cpu;
pub mod dev_env;
pub mod display;
pub mod gpu;
pub mod host;
pub mod memory;
pub mod network;
pub mod os;
pub mod peripherals;
pub mod sensors;
pub mod storage;

use crate::model::{
    BatteryInfo, CpuInfo, DevEnvInfo, DisplayInfo, GpuInfo, HostInfo, MemoryInfo, MotherboardInfo,
    NetworkInfo, OsInfo, PeripheralInfo, SensorReading, StorageInfo, SystemSnapshot,
};

/// 快照由这些相互独立的模块组成。界面可以逐个模块采集，哪个先完成就先展示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Module {
    Host,
    Os,
    Cpu,
    Gpu,
    Memory,
    Storage,
    Motherboard,
    Network,
    Display,
    Sensors,
    Battery,
    Peripherals,
    DevEnv,
}

impl Module {
    pub const ALL: [Module; 13] = [
        Module::Host,
        Module::Os,
        Module::Cpu,
        Module::Gpu,
        Module::Memory,
        Module::Storage,
        Module::Motherboard,
        Module::Network,
        Module::Display,
        Module::Sensors,
        Module::Battery,
        Module::Peripherals,
        Module::DevEnv,
    ];
}

/// 一个模块的采集结果。
#[derive(Debug, Clone)]
pub enum ModuleData {
    Host(HostInfo),
    Os(OsInfo),
    Cpu(CpuInfo),
    Gpus(Vec<GpuInfo>),
    Memory(MemoryInfo),
    Storages(Vec<StorageInfo>),
    Motherboard(Option<MotherboardInfo>),
    Network(NetworkInfo),
    Displays(Vec<DisplayInfo>),
    Sensors(Vec<SensorReading>),
    Battery(Option<BatteryInfo>),
    Peripherals(Vec<PeripheralInfo>),
    DevEnv(DevEnvInfo),
}

/// 采集单个模块。阻塞调用，请放在后台线程。
///
/// 采集代码要和各平台的系统接口、外部命令的输出打交道，任何一处意料之外都可能 panic。
/// 这里把它挡在模块边界上，按「什么都没采到」处理：一个模块出错不该影响其它模块，
/// 更不该让整轮刷新停在半路。
pub fn collect(module: Module) -> ModuleData {
    crate::util::guarded(&format!("{module:?} collection"), || {
        collect_unchecked(module)
    })
    .unwrap_or_else(|| empty(module))
}

fn empty(module: Module) -> ModuleData {
    match module {
        Module::Host => ModuleData::Host(HostInfo::default()),
        Module::Os => ModuleData::Os(OsInfo::default()),
        Module::Cpu => ModuleData::Cpu(CpuInfo::default()),
        Module::Gpu => ModuleData::Gpus(Vec::new()),
        Module::Memory => ModuleData::Memory(MemoryInfo::default()),
        Module::Storage => ModuleData::Storages(Vec::new()),
        Module::Motherboard => ModuleData::Motherboard(None),
        Module::Network => ModuleData::Network(NetworkInfo::default()),
        Module::Display => ModuleData::Displays(Vec::new()),
        Module::Sensors => ModuleData::Sensors(Vec::new()),
        Module::Battery => ModuleData::Battery(None),
        Module::Peripherals => ModuleData::Peripherals(Vec::new()),
        Module::DevEnv => ModuleData::DevEnv(DevEnvInfo::default()),
    }
}

fn collect_unchecked(module: Module) -> ModuleData {
    match module {
        Module::Host => ModuleData::Host(host::collect()),
        Module::Os => ModuleData::Os(os::collect()),
        Module::Cpu => ModuleData::Cpu(cpu::collect()),
        Module::Gpu => ModuleData::Gpus(gpu::collect()),
        Module::Memory => ModuleData::Memory(memory::collect()),
        Module::Storage => ModuleData::Storages(storage::collect()),
        Module::Motherboard => ModuleData::Motherboard(crate::platform::motherboard()),
        Module::Network => ModuleData::Network(network::collect()),
        Module::Display => ModuleData::Displays(display::collect()),
        Module::Sensors => ModuleData::Sensors(sensors::collect()),
        Module::Battery => ModuleData::Battery(battery::collect()),
        Module::Peripherals => ModuleData::Peripherals(peripherals::collect()),
        Module::DevEnv => ModuleData::DevEnv(dev_env::collect()),
    }
}

impl SystemSnapshot {
    /// 合入一个模块的结果。
    ///
    /// CPU 温度来自传感器模块，两者到达顺序不定，所以任何一方到达时都重新关联一次。
    pub fn apply(&mut self, data: ModuleData) {
        match data {
            ModuleData::Host(v) => self.host = v,
            ModuleData::Os(v) => self.os = v,
            ModuleData::Cpu(v) => {
                self.cpu = v;
                cpu::enrich_with_sensors(&mut self.cpu, &self.sensors);
            }
            ModuleData::Gpus(v) => self.gpus = v,
            ModuleData::Memory(v) => self.memory = v,
            ModuleData::Storages(v) => self.storages = v,
            ModuleData::Motherboard(v) => self.motherboard = v,
            ModuleData::Network(v) => {
                // 公网 IP 是用户显式开启后才查询的，刷新网卡时保留已有结果。
                let public_ip = self.network.public_ip.take();
                self.network = v;
                if self.network.public_ip.is_none() {
                    self.network.public_ip = public_ip;
                }
            }
            ModuleData::Displays(v) => self.displays = v,
            ModuleData::Sensors(v) => {
                self.sensors = v;
                cpu::enrich_with_sensors(&mut self.cpu, &self.sensors);
            }
            ModuleData::Battery(v) => self.battery = v,
            ModuleData::Peripherals(v) => self.peripherals = v,
            ModuleData::DevEnv(v) => self.dev_env = v,
        }
    }
}

/// 并行采集所有模块并合成完整快照，附带磁盘 / 网卡 / GPU 的实时速率。
/// 耗时约等于最慢的那个模块，而不是它们的总和。
pub fn collect_snapshot() -> SystemSnapshot {
    // 先建立速率基线：后面各模块的耗时本身就是速率的采样窗口。
    let mut sampler = crate::monitor::Sampler::new();
    let mut snapshot = SystemSnapshot::default();

    // CPU 使用率要在"我们自己还没开始 spawn 一堆子进程"时测，否则会把自身探测的开销算进去。
    snapshot.apply(collect(Module::Cpu));
    std::thread::scope(|s| {
        let handles: Vec<_> = Module::ALL
            .into_iter()
            .filter(|m| *m != Module::Cpu)
            .map(|m| s.spawn(move || collect(m)))
            .collect();
        for h in handles {
            if let Ok(data) = h.join() {
                snapshot.apply(data);
            }
        }
    });

    snapshot.merge_tick(&sampler.sample());
    snapshot.timestamp = crate::util::now_ms();
    snapshot
}
