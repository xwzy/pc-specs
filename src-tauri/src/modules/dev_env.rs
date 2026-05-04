use crate::model::{DevEnvInfo, RuntimeInfo};
use std::process::Command;

const ENV_KEYS_OF_INTEREST: &[&str] = &[
    "PATH",
    "SHELL",
    "LANG",
    "LC_ALL",
    "EDITOR",
    "TERM",
    "TERM_PROGRAM",
    "GOPATH",
    "GOROOT",
    "JAVA_HOME",
    "ANDROID_HOME",
    "NODE_VERSION",
    "PYTHONPATH",
    "RUSTUP_HOME",
    "CARGO_HOME",
    "VIRTUAL_ENV",
    "CONDA_DEFAULT_ENV",
    "PNPM_HOME",
    "DOCKER_HOST",
];

pub fn collect() -> DevEnvInfo {
    let lang_specs: &[(&str, &[&str])] = &[
        ("node", &["--version"]),
        ("python3", &["--version"]),
        ("python", &["--version"]),
        ("rustc", &["--version"]),
        ("go", &["version"]),
        ("java", &["-version"]),
        ("ruby", &["--version"]),
        ("php", &["--version"]),
        ("deno", &["--version"]),
        ("bun", &["--version"]),
    ];
    let pm_specs: &[(&str, &[&str])] = &[
        ("npm", &["--version"]),
        ("pnpm", &["--version"]),
        ("yarn", &["--version"]),
        ("pip", &["--version"]),
        ("pip3", &["--version"]),
        ("cargo", &["--version"]),
        ("brew", &["--version"]),
        ("apt", &["--version"]),
        ("winget", &["--version"]),
        ("choco", &["--version"]),
    ];
    let vcs_specs: &[(&str, &[&str])] = &[
        ("git", &["--version"]),
        ("hg", &["--version"]),
        ("svn", &["--version"]),
    ];
    let editor_specs: &[(&str, &[&str])] = &[
        ("code", &["--version"]),
        ("cursor", &["--version"]),
        ("vim", &["--version"]),
        ("nvim", &["--version"]),
        ("emacs", &["--version"]),
    ];
    let container_specs: &[(&str, &[&str])] = &[
        ("docker", &["--version"]),
        ("podman", &["--version"]),
        // `kubectl version --client --short` 在 1.28 deprecated，1.29+ 移除；
        // `--client` 输出已经简洁，不需要 --short。
        ("kubectl", &["version", "--client"]),
        ("colima", &["version"]),
        ("nerdctl", &["--version"]),
        ("helm", &["version", "--short"]),
    ];
    let shell_specs: &[(&str, &[&str])] = &[
        ("bash", &["--version"]),
        ("zsh", &["--version"]),
        ("fish", &["--version"]),
        ("pwsh", &["--version"]),
        ("powershell", &["--version"]),
    ];

    let languages = probe_parallel(lang_specs);
    let package_managers = probe_parallel(pm_specs);
    let vcs = probe_parallel(vcs_specs);
    let editors = probe_parallel(editor_specs);
    let containers = probe_parallel(container_specs);
    let shells = probe_parallel(shell_specs);

    let env_keys = ENV_KEYS_OF_INTEREST
        .iter()
        .filter(|k| std::env::var(*k).is_ok())
        .map(|k| k.to_string())
        .collect();

    DevEnvInfo {
        languages: filter_present(languages),
        package_managers: filter_present(package_managers),
        vcs: filter_present(vcs),
        editors: filter_present(editors),
        containers: filter_present(containers),
        shells: filter_present(shells),
        env_keys,
    }
}

fn probe_parallel(specs: &[(&str, &[&str])]) -> Vec<RuntimeInfo> {
    use std::thread;
    let handles: Vec<_> = specs
        .iter()
        .map(|(name, args)| {
            let name = name.to_string();
            let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            thread::spawn(move || {
                let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                probe(&name, &args_ref)
            })
        })
        .collect();
    handles
        .into_iter()
        .map(|h| h.join().unwrap_or_else(|_| RuntimeInfo {
            name: "<panic>".to_string(),
            version: None,
            path: None,
        }))
        .collect()
}

fn filter_present(rs: Vec<RuntimeInfo>) -> Vec<RuntimeInfo> {
    rs.into_iter().filter(|r| r.version.is_some()).collect()
}

fn probe(name: &str, args: &[&str]) -> RuntimeInfo {
    let out = Command::new(name).args(args).output();
    let path = which(name);
    match out {
        Ok(o) if o.status.success() => {
            let mut combined = String::new();
            combined.push_str(&String::from_utf8_lossy(&o.stdout));
            if combined.trim().is_empty() {
                combined.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            let line = combined.lines().next().unwrap_or("").trim().to_string();
            let version = if line.is_empty() { None } else { Some(line) };
            RuntimeInfo {
                name: name.to_string(),
                version,
                path,
            }
        }
        _ => RuntimeInfo {
            name: name.to_string(),
            version: None,
            path,
        },
    }
}

fn which(name: &str) -> Option<String> {
    let cmd = if cfg!(target_os = "windows") { "where" } else { "which" };
    let out = Command::new(cmd).arg(name).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if s.is_empty() { None } else { Some(s) }
}
