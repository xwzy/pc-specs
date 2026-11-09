//! 项目里的构建产物：`node_modules`、Rust 的 `target`、Python 虚拟环境……都是工具随时能重新生成的东西，
//! 一个早就不碰的项目往往就占着几个 GB。
//!
//! 和别的几类不同，它们的位置不是写死的，要到放代码的文件夹里去找；所以多设了几道保险：
//! - 文件夹叫对名字还不够，旁边得有对应工具的项目文件（`node_modules` 旁边要有 `package.json`）；
//!   虚拟环境还要里面有 `pyvenv.cfg`、旁边有依赖清单（没有清单就没法照原样重建）。认不出来的同名文件夹一律不碰。
//! - 项目要闲置两周以上：产物里、项目里、Git 记录里，任何一处两周内动过，整个项目都不碰。
//!   真正清理前还会再核对一遍，免得量完到点下去之间项目又被用起来了。
//! - 只在放代码的几个常见文件夹里找，找到产物就不再往它里面找；不跟随符号链接。

use super::{Kind, Root, Spec, deep_enough};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant, SystemTime};

/// 项目闲置多久才动它。
const IDLE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
/// 找项目最多往下走几层（从放代码的文件夹算起）。
const MAX_DEPTH: usize = 6;
/// 找项目最多看多少个文件夹、最多花多久，免得撞上巨大的目录树卡很久；到了就用已经找到的。
const MAX_DIRS: usize = 200_000;
const BUDGET: Duration = Duration::from_secs(30);
/// 判断项目有没有被动过时：往项目里看几层、最多看多少项；多到看不完就当作动过。
const WATCH_DEPTH: usize = 3;
const WATCH_LIMIT: usize = 3_000;
/// 产物自己第一层最多看多少项。
const FIRST_LEVEL: usize = 20_000;
/// 找项目所在的 Git 仓库，最多往上找几层。
const GIT_LEVELS: usize = 8;

/// 一种构建产物。
struct Artifact {
    /// 文件夹名字。
    dir: &'static str,
    /// 它旁边得有这些项目文件之一。
    beside: &'static [&'static str],
    /// 如果有，它里面还得有这个文件。
    inside: Option<&'static str>,
}

impl Artifact {
    const fn beside(dir: &'static str, files: &'static [&'static str]) -> Artifact {
        Artifact {
            dir,
            beside: files,
            inside: None,
        }
    }

    /// Python 虚拟环境：里面有 `pyvenv.cfg`，旁边有依赖清单。
    const fn venv(dir: &'static str) -> Artifact {
        Artifact {
            dir,
            beside: PYTHON_PROJECT,
            inside: Some("pyvenv.cfg"),
        }
    }

    /// `siblings` 是产物所在文件夹里所有项目的名字，`path` 是产物自己。
    fn matches(&self, siblings: &HashSet<String>, path: &Path) -> bool {
        self.beside.iter().any(|file| siblings.contains(*file))
            && self
                .inside
                .is_none_or(|file| path.join(file).symlink_metadata().is_ok())
    }
}

const PYTHON_PROJECT: &[&str] = &[
    "requirements.txt",
    "pyproject.toml",
    "Pipfile",
    "poetry.lock",
    "uv.lock",
    "setup.py",
    "environment.yml",
];

const GRADLE: &[&str] = &[
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
];

const ARTIFACTS: &[Artifact] = &[
    // JavaScript
    Artifact::beside("node_modules", &["package.json"]),
    Artifact::beside(".next", &["package.json"]),
    Artifact::beside(".nuxt", &["package.json"]),
    Artifact::beside(".svelte-kit", &["package.json"]),
    Artifact::beside(".parcel-cache", &["package.json"]),
    Artifact::beside(".turbo", &["package.json"]),
    Artifact::beside(".angular", &["angular.json"]),
    // Rust、Maven、sbt
    Artifact::beside("target", &["Cargo.toml", "pom.xml", "build.sbt"]),
    // Gradle / Android、Flutter / Dart
    Artifact::beside(".gradle", GRADLE),
    Artifact::beside(
        "build",
        &["build.gradle", "build.gradle.kts", "pubspec.yaml"],
    ),
    Artifact::beside(".dart_tool", &["pubspec.yaml"]),
    // Swift Package Manager
    Artifact::beside(".build", &["Package.swift"]),
    // Python
    Artifact::venv(".venv"),
    Artifact::venv("venv"),
    Artifact::venv("env"),
    Artifact::beside(".tox", &["tox.ini", "pyproject.toml", "setup.cfg"]),
    Artifact::beside(".nox", &["noxfile.py"]),
    // 其他
    Artifact::beside("_build", &["mix.exs"]),
    Artifact::beside(".stack-work", &["stack.yaml"]),
    Artifact::beside("zig-cache", &["build.zig"]),
    Artifact::beside(".zig-cache", &["build.zig"]),
];

/// 这台机器上，闲置项目里的构建产物。一个也没找到就是 `None`。
/// 要走遍放代码的文件夹，可能要几秒，别在界面线程里调。
pub fn projects(cancel: &AtomicBool) -> Option<Spec> {
    let home = dirs::home_dir()?;
    let mut roots = discover(&folders(&home), cancel);
    roots.retain(|root| deep_enough(&root.path));
    (!roots.is_empty()).then_some(Spec {
        kind: Kind::Projects,
        roots,
    })
}

/// 放代码的常见文件夹，只留下真的存在、且不是符号链接的（链接指向的地方可能已经在别的文件夹里，会算两遍）。
/// macOS 和 Windows 的文件名不分大小写，一种写法就够；Linux 区分，两种都列。
fn folders(home: &Path) -> Vec<PathBuf> {
    let mut names = vec![
        "Developer",
        "Projects",
        "Code",
        "dev",
        "workspace",
        "src",
        "repos",
        "git",
        "github",
        "work",
    ];
    if cfg!(target_os = "linux") {
        names.extend([
            "developer",
            "projects",
            "code",
            "Dev",
            "Workspace",
            "Src",
            "Repos",
            "Git",
            "GitHub",
            "Work",
        ]);
    }
    let mut folders: Vec<PathBuf> = names.into_iter().map(|name| home.join(name)).collect();
    // Visual Studio 默认把项目放在这里。
    folders.push(home.join("source/repos"));
    // 文档和桌面也常有人放代码。它们可能被重定向到别处（比如 OneDrive），所以问系统要。
    folders.extend(dirs::document_dir());
    folders.extend(dirs::desktop_dir());
    folders
        .retain(|folder| fs::symlink_metadata(folder).is_ok_and(|meta| meta.file_type().is_dir()));
    folders
}

/// 进不进这个文件夹往下找：隐藏的、应用包、系统和应用自己的数据都不进。
fn worth_entering(name: &str) -> bool {
    const SKIP_EXTENSIONS: [&str; 6] = [
        ".app",
        ".photoslibrary",
        ".xcodeproj",
        ".xcworkspace",
        ".framework",
        ".sparsebundle",
    ];
    !name.starts_with('.')
        && !matches!(
            name,
            "node_modules" | "Library" | "Applications" | "AppData"
        )
        && !SKIP_EXTENSIONS
            .iter()
            .any(|extension| name.to_lowercase().ends_with(extension))
}

/// 在这些文件夹里找构建产物（不管项目闲不闲置，那一关在清理和计算时才过）。
fn discover(folders: &[PathBuf], cancel: &AtomicBool) -> Vec<Root> {
    let deadline = Instant::now() + BUDGET;
    let mut visited = 0usize;
    let mut roots: Vec<Root> = Vec::new();
    'search: for folder in folders {
        let mut pending = vec![(folder.clone(), 0usize)];
        while let Some((dir, depth)) = pending.pop() {
            if cancel.load(Relaxed) || visited >= MAX_DIRS || Instant::now() > deadline {
                break 'search;
            }
            visited += 1;
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            let mut names: HashSet<String> = HashSet::new();
            let mut subdirs: Vec<(String, PathBuf)> = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                // `file_type` 不跟随符号链接：指向文件夹的链接不是文件夹。
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    subdirs.push((name.clone(), entry.path()));
                }
                names.insert(name);
            }
            for (name, path) in subdirs {
                if let Some(artifact) = ARTIFACTS
                    .iter()
                    .find(|artifact| artifact.dir == name && artifact.matches(&names, &path))
                {
                    let project = dir.file_name().map_or_else(
                        || dir.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    );
                    roots.push(
                        Root::new(path)
                            .labeled(format!("{project} · {}", artifact.dir))
                            .in_project(dir.clone()),
                    );
                } else if depth < MAX_DEPTH && worth_entering(&name) {
                    pending.push((path, depth + 1));
                }
            }
        }
    }
    roots.sort_by(|a, b| a.path.cmp(&b.path));
    roots.dedup_by(|a, b| a.path == b.path);
    roots
}

/// 项目这两周有没有被动过：动过就整个不碰。
pub(super) fn recently_touched(project: &Path, artifact: &Path) -> bool {
    SystemTime::now()
        .checked_sub(IDLE)
        .is_none_or(|since| touched_since(project, artifact, since))
}

/// 项目（连同它的这份产物和 Git 记录）自 `since` 起有没有被动过。查不清楚的一律算动过。
fn touched_since(project: &Path, artifact: &Path, since: SystemTime) -> bool {
    let fresh = |meta: &fs::Metadata| meta.modified().map_or(true, |time| time >= since);

    // 产物自己和它的第一层：装了新依赖、重新构建过，都会改到这里。
    if fs::symlink_metadata(artifact).map_or(true, |meta| fresh(&meta)) {
        return true;
    }
    if let Ok(entries) = fs::read_dir(artifact) {
        for entry in entries.take(FIRST_LEVEL).flatten() {
            if entry.metadata().map_or(true, |meta| fresh(&meta)) {
                return true;
            }
        }
    }

    // 项目里的文件：往下几层，其他产物和 `.git` 不算（后者下面单独看）。
    let mut seen = 0usize;
    let mut pending = vec![(project.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                return true;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == ".git" || ARTIFACTS.iter().any(|artifact| artifact.dir == name) {
                    continue;
                }
            }
            seen += 1;
            if seen > WATCH_LIMIT || entry.metadata().map_or(true, |meta| fresh(&meta)) {
                return true;
            }
            if kind.is_dir() && depth < WATCH_DEPTH {
                pending.push((entry.path(), depth + 1));
            }
        }
    }

    // Git 记录：提交、切换分支、拉取都会改到这几个文件。找到最近的仓库为止；
    // 个人文件夹本身不算（有人拿它放点文件的版本管理，那会让所有项目都显得很新）。
    let home = dirs::home_dir();
    let mut here = Some(project);
    for _ in 0..GIT_LEVELS {
        let Some(dir) = here else { break };
        if home.as_deref() == Some(dir) {
            break;
        }
        let git = dir.join(".git");
        if git.is_dir() {
            return ["index", "HEAD", "logs/HEAD"]
                .iter()
                .any(|file| fs::metadata(git.join(file)).is_ok_and(|meta| fresh(&meta)));
        }
        here = dir.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let path = std::env::temp_dir().join(format!("pcs-proj-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Scratch(path)
        }

        fn write(&self, relative: &str, bytes: usize) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, vec![1u8; bytes]).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    fn found(dir: &Scratch) -> Vec<PathBuf> {
        discover(std::slice::from_ref(&dir.0), &never())
            .into_iter()
            .map(|root| root.path.strip_prefix(&dir.0).unwrap().to_path_buf())
            .collect()
    }

    /// 比现在晚一天：此刻已经在磁盘上的东西，都算「一天前就没动过」。
    fn tomorrow() -> SystemTime {
        SystemTime::now() + 24 * 60 * 60 * Duration::from_secs(1)
    }

    /// 把文件的修改时间拨到后天：相对 [`tomorrow`] 它就是「刚刚才动过」。
    fn touch(path: &Path) {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() + 2 * 24 * 60 * 60 * Duration::from_secs(1))
            .unwrap();
    }

    #[test]
    fn artifacts_are_only_recognised_next_to_their_project_files() {
        let dir = Scratch::new("markers");
        dir.write("web/package.json", 10);
        dir.write("web/node_modules/left-pad/index.js", 100);
        dir.write("web/.next/cache/a.bin", 100);
        dir.write("rust/Cargo.toml", 10);
        dir.write("rust/target/debug/app", 100);
        dir.write("java/pom.xml", 10);
        dir.write("java/target/classes/A.class", 100);
        // 没有项目文件的同名文件夹：不是产物。
        dir.write("plain/node_modules/x/y.js", 100);
        dir.write("plain/target/notes.txt", 100);
        dir.write("plain/build/out.txt", 100);
        // 项目文件对不上：Cargo 的 target 旁边得是 Cargo.toml，npm 的 package.json 不行。
        dir.write("mixed/package.json", 10);
        dir.write("mixed/target/debug/app", 100);

        assert_eq!(
            found(&dir),
            [
                "java/target",
                "rust/target",
                "web/.next",
                "web/node_modules"
            ]
            .map(PathBuf::from)
        );
    }

    #[test]
    fn virtualenvs_need_both_the_config_inside_and_a_requirements_file_beside() {
        let dir = Scratch::new("venv");
        dir.write("app/requirements.txt", 10);
        dir.write("app/.venv/pyvenv.cfg", 10);
        dir.write("app/.venv/lib/site.py", 100);
        // 有虚拟环境、没有依赖清单：没法照原样重建，不碰。
        dir.write("adhoc/venv/pyvenv.cfg", 10);
        // 有清单、但文件夹里不是虚拟环境。
        dir.write("data/pyproject.toml", 10);
        dir.write("data/env/readme.txt", 10);
        assert_eq!(found(&dir), [PathBuf::from("app/.venv")]);
    }

    #[test]
    fn the_search_does_not_go_into_artifacts_or_hidden_and_app_folders() {
        let dir = Scratch::new("skip");
        dir.write("web/package.json", 10);
        // 产物里面的东西不再找。
        dir.write("web/node_modules/dep/package.json", 10);
        dir.write("web/node_modules/dep/node_modules/inner/a.js", 10);
        // 隐藏文件夹、应用包里的不找。
        dir.write(".hidden/proj/package.json", 10);
        dir.write(".hidden/proj/node_modules/a.js", 10);
        dir.write("Some.app/proj/package.json", 10);
        dir.write("Some.app/proj/node_modules/a.js", 10);
        // 嵌套的项目（monorepo）：每个包各算各的。
        dir.write("mono/apps/site/package.json", 10);
        dir.write("mono/apps/site/node_modules/a.js", 10);
        // 太深的不找。
        let deep = "a/b/c/d/e/f/g/h";
        dir.write(&format!("{deep}/package.json"), 10);
        dir.write(&format!("{deep}/node_modules/a.js"), 10);

        assert_eq!(
            found(&dir),
            ["mono/apps/site/node_modules", "web/node_modules"].map(PathBuf::from)
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_to_folders_are_not_artifacts_and_not_followed() {
        let dir = Scratch::new("links");
        dir.write("shared/lib/a.js", 100);
        dir.write("web/package.json", 10);
        std::os::unix::fs::symlink(dir.0.join("shared"), dir.0.join("web/node_modules")).unwrap();
        std::os::unix::fs::symlink(dir.0.join("shared"), dir.0.join("alias")).unwrap();
        assert!(found(&dir).is_empty());
    }

    #[test]
    fn a_project_counts_as_touched_when_anything_in_it_changed_recently() {
        let dir = Scratch::new("touched");
        let artifact = dir.0.join("app/node_modules");
        let project = dir.0.join("app");
        let since = tomorrow();
        dir.write("app/package.json", 10);
        let source = dir.write("app/src/deep/main.js", 10);
        dir.write("app/node_modules/dep/index.js", 10);
        assert!(!touched_since(&project, &artifact, since));

        // 源码改过：动过。
        touch(&source);
        assert!(touched_since(&project, &artifact, since));
    }

    #[test]
    fn an_install_or_a_build_touching_the_artifacts_first_level_counts_too() {
        let dir = Scratch::new("signals");
        let project = dir.0.join("app");
        let since = tomorrow();
        dir.write("app/package.json", 10);
        dir.write("app/node_modules/dep/index.js", 10);
        let lock = dir.write("app/node_modules/.package-lock.json", 10);
        dir.write("app/target/debug/app", 10);
        dir.write("app/Cargo.toml", 10);
        let info = dir.write("app/target/.rustc_info.json", 10);
        assert!(!touched_since(
            &project,
            &project.join("node_modules"),
            since
        ));
        assert!(!touched_since(&project, &project.join("target"), since));

        // 重新装过依赖：npm 每次都会重写它。
        touch(&lock);
        assert!(touched_since(
            &project,
            &project.join("node_modules"),
            since
        ));
        // 别的产物里的动静不算：它们在项目里是被跳过的。
        assert!(!touched_since(&project, &project.join("target"), since));
        touch(&info);
        assert!(touched_since(&project, &project.join("target"), since));
    }

    #[test]
    fn git_activity_is_read_from_the_nearest_repository() {
        let dir = Scratch::new("git");
        let repo = dir.0.join("repo");
        let project = repo.join("packages/web");
        let artifact = project.join("node_modules");
        let since = tomorrow();
        dir.write("repo/packages/web/package.json", 10);
        dir.write("repo/packages/web/node_modules/a.js", 10);
        dir.write("repo/.git/HEAD", 10);
        dir.write("repo/.git/index", 10);
        // `.git` 里的东西不当作项目文件，光有它们不算动过。
        assert!(!touched_since(&project, &artifact, since));
        // 提交、切换分支会改到 `.git/index`：仓库里的所有包都算动过。
        touch(&repo.join(".git/index"));
        assert!(touched_since(&project, &artifact, since));
    }

    #[test]
    fn a_project_with_too_many_entries_to_check_is_left_alone() {
        let dir = Scratch::new("big");
        let project = dir.0.join("huge");
        dir.write("huge/package.json", 10);
        dir.write("huge/node_modules/a.js", 10);
        for index in 0..=WATCH_LIMIT {
            File::create(dir.0.join(format!("huge/f{index}"))).unwrap();
        }
        assert!(touched_since(
            &project,
            &project.join("node_modules"),
            tomorrow()
        ));
    }

    #[test]
    fn a_missing_artifact_or_unreadable_time_never_counts_as_idle() {
        let dir = Scratch::new("missing");
        dir.write("app/package.json", 10);
        assert!(touched_since(
            &dir.0.join("app"),
            &dir.0.join("app/node_modules"),
            tomorrow()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn only_an_idle_project_is_measured_and_cleaned_and_it_is_checked_again_at_the_last_moment() {
        use crate::clean::{Cleaned, Progress, clean, measure};

        // 把整个项目拨到一个月前。
        fn age(path: &Path) {
            let month_ago = SystemTime::now() - 30 * 24 * 60 * 60 * Duration::from_secs(1);
            for entry in fs::read_dir(path).unwrap().flatten() {
                if entry.file_type().unwrap().is_dir() {
                    age(&entry.path());
                }
                File::open(entry.path())
                    .unwrap()
                    .set_modified(month_ago)
                    .unwrap();
            }
            File::open(path).unwrap().set_modified(month_ago).unwrap();
        }

        let dir = Scratch::new("clean");
        dir.write("old/package.json", 10);
        dir.write("old/src/index.js", 10);
        dir.write("old/node_modules/dep/index.js", 200_000);
        dir.write("old/node_modules/other/index.js", 100_000);
        dir.write("busy/package.json", 10);
        dir.write("busy/node_modules/dep/index.js", 200_000);
        age(&dir.0.join("old"));
        // busy 没有拨：刚刚才建，算正在用。

        let roots = discover(std::slice::from_ref(&dir.0), &never());
        assert_eq!(roots.len(), 2);
        let spec = Spec {
            kind: Kind::Projects,
            roots,
        };

        let measured = measure(&spec, &never());
        assert!(measured.bytes >= 300_000 && measured.bytes < 300_000 + 128 * 1024);
        assert_eq!(measured.biggest.len(), 1);
        assert_eq!(measured.biggest[0].0, "old · node_modules");
        assert_eq!(
            measured.main.as_deref(),
            Some(dir.0.join("old/node_modules").as_path())
        );

        // 量完之后，old 又被动了一下：清理的时候再核对，就不清了。
        let progress = Progress::default();
        let source = dir.0.join("old/src/index.js");
        let before = fs::metadata(&source).unwrap().modified().unwrap();
        File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(SystemTime::now())
            .unwrap();
        assert_eq!(clean(&spec, &progress, &never()), Cleaned::default());
        assert!(dir.0.join("old/node_modules/dep/index.js").exists());
        assert!(dir.0.join("busy/node_modules/dep/index.js").exists());

        // 放回去之后（又闲置了），才清；只清产物里面，产物本身和源码都在。
        File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(before)
            .unwrap();
        let cleaned = clean(&spec, &progress, &never());
        assert_eq!(
            cleaned,
            Cleaned {
                removed: 2,
                failed: 0
            }
        );
        assert!(
            fs::read_dir(dir.0.join("old/node_modules"))
                .unwrap()
                .next()
                .is_none()
        );
        assert!(source.is_file() && dir.0.join("old/package.json").is_file());
        assert!(dir.0.join("busy/node_modules/dep/index.js").exists());
        assert!(progress.freed.load(Relaxed) >= 300_000);
    }

    #[test]
    fn every_artifact_names_what_must_be_beside_it_and_search_folders_are_plain_directories() {
        for artifact in ARTIFACTS {
            assert!(
                !artifact.beside.is_empty(),
                "{} could be recognised by name alone",
                artifact.dir
            );
            assert!(!artifact.dir.contains('/') && !artifact.dir.is_empty());
        }
        let mut names: Vec<&str> = ARTIFACTS.iter().map(|artifact| artifact.dir).collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        assert_eq!(names.len(), total, "an artifact is listed twice");

        let home = Scratch::new("home");
        fs::create_dir_all(home.0.join("Projects")).unwrap();
        fs::create_dir_all(home.0.join("elsewhere")).unwrap();
        fs::write(home.0.join("Code"), b"a file, not a folder").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(home.0.join("elsewhere"), home.0.join("work")).unwrap();
        let folders = folders(&home.0);
        assert!(folders.contains(&home.0.join("Projects")));
        assert!(!folders.contains(&home.0.join("Code")));
        assert!(!folders.contains(&home.0.join("work")));
        assert!(!folders.contains(&home.0.join("elsewhere")));

        assert!(worth_entering("my-app"));
        assert!(!worth_entering(".cache"));
        assert!(!worth_entering("Photos.photoslibrary"));
        assert!(!worth_entering("Some.APP"));
        assert!(!worth_entering("Library"));
        assert!(!worth_entering("node_modules"));
    }
}
