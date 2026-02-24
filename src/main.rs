use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};

use nojson::RawJson;

#[derive(Debug)]
enum Error {
    Io(std::io::Error),
    Utf8(std::string::FromUtf8Error),
    Args(noargs::Error),
    Json(nojson::JsonParseError),
    Message(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O エラー: {e}"),
            Self::Utf8(e) => write!(f, "UTF-8 エラー: {e}"),
            Self::Args(e) => write!(f, "引数エラー: {e:?}"),
            Self::Json(e) => write!(f, "JSON 解析エラー: {e}"),
            Self::Message(msg) => f.write_str(msg),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<std::string::FromUtf8Error> for Error {
    fn from(value: std::string::FromUtf8Error) -> Self {
        Self::Utf8(value)
    }
}

impl From<noargs::Error> for Error {
    fn from(value: noargs::Error) -> Self {
        Self::Args(value)
    }
}

impl From<nojson::JsonParseError> for Error {
    fn from(value: nojson::JsonParseError) -> Self {
        Self::Json(value)
    }
}

type Result<T> = std::result::Result<T, Error>;
type JsonResult<T> = std::result::Result<T, nojson::JsonParseError>;

#[derive(Debug, Clone)]
struct CliArgs {
    config_path: PathBuf,
}

#[derive(Debug, Clone)]
struct SysrootConfig {
    name: String,
    arch: String,
    rust_target: String,
    linker: String,
    packages: Vec<String>,
    repos: Vec<RepoSpec>,
}

#[derive(Debug, Clone)]
struct RepoSpec {
    url: String,
    suites: Vec<String>,
    components: Vec<String>,
}

trait RawJsonTryIntoExt<'text, 'raw> {
    fn parse_into<T>(self) -> JsonResult<T>
    where
        T: TryFrom<nojson::RawJsonValue<'text, 'raw>, Error = nojson::JsonParseError>;
}

impl<'text, 'raw> RawJsonTryIntoExt<'text, 'raw> for nojson::RawJsonValue<'text, 'raw> {
    fn parse_into<T>(self) -> JsonResult<T>
    where
        T: TryFrom<nojson::RawJsonValue<'text, 'raw>, Error = nojson::JsonParseError>,
    {
        self.try_into()
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for SysrootConfig {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        let name = required_non_empty_string_member(value, "name", "name")?;
        validate_name(value.to_member("name")?.required()?, &name)?;

        let arch = required_non_empty_string_member(value, "arch", "arch")?;
        let rust_target = required_non_empty_string_member(value, "rust_target", "rust_target")?;
        let linker = required_non_empty_string_member(value, "linker", "linker")?;

        let packages = required_non_empty_string_array_member(value, "packages", "packages")?;

        let repos_value = value.to_member("repos")?.required()?;
        let repos = repos_value
            .to_array()?
            .map(|item| item.parse_into())
            .collect::<JsonResult<Vec<RepoSpec>>>()?;
        if repos.is_empty() {
            return Err(repos_value.invalid("repos が空です"));
        }

        Ok(Self {
            name,
            arch,
            rust_target,
            linker,
            packages,
            repos,
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for RepoSpec {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> JsonResult<Self> {
        let url = required_non_empty_string_member(value, "url", "repos[].url")?;
        let suites = required_non_empty_string_array_member(value, "suites", "repos[].suites")?;
        let components =
            required_non_empty_string_array_member(value, "components", "repos[].components")?;

        Ok(Self {
            url,
            suites,
            components,
        })
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args = parse_args()?;
    let config = load_config(&args.config_path)?;

    let target_directory = load_target_directory_from_metadata()?;
    let target_dir = Path::new(&target_directory);

    let bundle_dir = target_dir.join("shiguredo-sysroot").join(&config.name);
    let sysroot_dir = bundle_dir.join("sysroot");
    let workbase = bundle_dir.join("work");

    build_sysroot(&config, &config.arch, &sysroot_dir, &workbase)?;

    let cwd = std::env::current_dir()?;
    update_cargo_config(
        &cwd,
        &sysroot_dir,
        &config.rust_target,
        &config.linker,
        &config.arch,
    )?;

    println!("Done.");
    println!("Target directory : {}", target_dir.display());
    println!("Sysroot          : {}", sysroot_dir.display());
    println!(
        "Cargo config     : {}",
        cwd.join(".cargo/config.toml").display()
    );

    Ok(())
}

fn parse_args() -> Result<CliArgs> {
    parse_args_from_argv(std::env::args().collect())
}

fn parse_args_from_argv(argv: Vec<String>) -> Result<CliArgs> {
    let mut args = noargs::RawArgs::new(normalize_argv_for_noargs(argv).into_iter());
    args.metadata_mut().app_name = "cargo shiguredo-sysroot";
    args.metadata_mut().app_description = "クロスコンパイル用 sysroot の生成と Cargo 設定更新";

    if noargs::VERSION_FLAG.take(&mut args).is_present() {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    noargs::HELP_FLAG.take_help(&mut args);

    let config_path: PathBuf = noargs::opt("config")
        .doc("設定 JSON ファイルのパス")
        .example("ubuntu-24.04_armv8.json")
        .take(&mut args)
        .then(|o| Ok::<_, &str>(PathBuf::from(o.value().to_string())))?;

    if let Some(help) = args.finish()? {
        print!("{help}");
        std::process::exit(0);
    }

    Ok(CliArgs { config_path })
}

fn normalize_argv_for_noargs(mut argv: Vec<String>) -> Vec<String> {
    if argv.get(1).is_some_and(|arg| arg == "shiguredo-sysroot") {
        argv.remove(1);
    }
    argv
}

fn load_config(path: &Path) -> Result<SysrootConfig> {
    let text = fs::read_to_string(path)?;
    parse_sysroot_config_text(&text)
}

fn parse_sysroot_config_text(text: &str) -> Result<SysrootConfig> {
    let json = RawJson::parse(text)?;
    let config: SysrootConfig = json.value().parse_into()?;
    Ok(config)
}

fn parse_string_array(value: nojson::RawJsonValue<'_, '_>, label: &str) -> JsonResult<Vec<String>> {
    value
        .to_array()?
        .map(|item| -> JsonResult<String> {
            let s: String = item.try_into()?;
            if s.is_empty() {
                return Err(item.invalid(format!("{label} に空文字列は指定できません")));
            }
            Ok(s)
        })
        .collect::<JsonResult<Vec<_>>>()
}

fn required_non_empty_string_member(
    value: nojson::RawJsonValue<'_, '_>,
    key: &str,
    label: &str,
) -> JsonResult<String> {
    let member = value.to_member(key)?.required()?;
    let s: String = member.try_into()?;
    if s.is_empty() {
        return Err(member.invalid(format!("{label} が空です")));
    }
    Ok(s)
}

fn required_non_empty_string_array_member(
    value: nojson::RawJsonValue<'_, '_>,
    key: &str,
    label: &str,
) -> JsonResult<Vec<String>> {
    let member = value.to_member(key)?.required()?;
    let items = parse_string_array(member, label)?;
    if items.is_empty() {
        return Err(member.invalid(format!("{label} が空です")));
    }
    Ok(items)
}

fn validate_name(value: nojson::RawJsonValue<'_, '_>, name: &str) -> JsonResult<()> {
    if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Ok(());
    }

    Err(value.invalid("name は [A-Za-z0-9._-]+ のみ指定できます"))
}

fn load_target_directory_from_metadata() -> Result<String> {
    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--no-deps")
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8(output.stderr)?;
        return Err(Error::Message(format!(
            "cargo metadata の実行に失敗しました: {stderr}"
        )));
    }

    let stdout = String::from_utf8(output.stdout)?;
    let json = RawJson::parse(&stdout)?;
    let target_directory: String = json
        .value()
        .to_member("target_directory")?
        .required()?
        .try_into()?;

    Ok(target_directory)
}

fn build_sysroot(
    config: &SysrootConfig,
    apt_arch: &str,
    output_dir: &Path,
    workbase: &Path,
) -> Result<()> {
    if output_dir.exists() {
        fs::remove_dir_all(output_dir)?;
    }
    fs::create_dir_all(output_dir)?;

    let workdir = workbase.join(format!("sysroot-apt-{apt_arch}"));
    fs::create_dir_all(workdir.join("state/lists/partial"))?;
    fs::create_dir_all(workdir.join("state/cache/archives/partial"))?;

    File::create(workdir.join("state/status"))?;

    let apt_conf = format!(
        "APT::Architecture \"{apt_arch}\";\nAPT::Architectures {{ \"{apt_arch}\"; }};\nAcquire::Languages \"none\";\n"
    );
    fs::write(workdir.join("apt.conf"), apt_conf)?;

    let mut sources = String::new();
    for repo in &config.repos {
        for suite in &repo.suites {
            append_repo_line(&mut sources, apt_arch, &repo.url, suite, &repo.components);
        }
    }
    fs::write(workdir.join("sources.list"), sources)?;

    let apt_opts = build_apt_options(&workdir);

    run_command(
        build_apt_command(&workdir, &apt_opts, &["update"]),
        "apt-get update",
    )?;

    let packages_with_arch: Vec<String> = config
        .packages
        .iter()
        .map(|p| {
            if p.contains(':') {
                p.clone()
            } else {
                format!("{p}:{apt_arch}")
            }
        })
        .collect();

    let mut install_args: Vec<OsString> = vec![
        OsString::from("-d"),
        OsString::from("-y"),
        OsString::from("-o"),
        OsString::from("APT::Get::Download-Only=true"),
        OsString::from("-o"),
        OsString::from("Debug::NoLocking=true"),
        OsString::from("install"),
    ];
    install_args.extend(packages_with_arch.iter().map(OsString::from));

    run_command(
        build_apt_command(&workdir, &apt_opts, &install_args),
        "apt-get install (download only)",
    )?;

    let debs = collect_deb_files(&workdir.join("state/cache/archives"))?;
    if debs.is_empty() {
        return Err(Error::Message(
            ".deb が 1 つもダウンロードされませんでした。repo/suites/components/arch を確認してください"
                .to_string(),
        ));
    }

    for deb in debs {
        let mut cmd = Command::new("dpkg-deb");
        cmd.arg("-x").arg(&deb).arg(output_dir);
        run_command(cmd, &format!("dpkg-deb -x {}", deb.display()))?;
    }

    ensure_usrmerge_symlinks(output_dir)?;
    fix_absolute_symlinks(output_dir)?;

    Ok(())
}

fn append_repo_line(buf: &mut String, arch: &str, url: &str, suite: &str, components: &[String]) {
    let comps = components.join(" ");
    buf.push_str(&format!("deb [arch={arch}] {url} {suite} {comps}\n"));
}

fn build_apt_options(workdir: &Path) -> Vec<OsString> {
    vec![
        OsString::from("-o"),
        OsString::from(format!("Dir::State={}", workdir.join("state").display())),
        OsString::from("-o"),
        OsString::from(format!(
            "Dir::State::status={}",
            workdir.join("state/status").display()
        )),
        OsString::from("-o"),
        OsString::from(format!(
            "Dir::Cache={}",
            workdir.join("state/cache").display()
        )),
        OsString::from("-o"),
        OsString::from(format!(
            "Dir::Etc::sourcelist={}",
            workdir.join("sources.list").display()
        )),
        OsString::from("-o"),
        OsString::from("Dir::Etc::sourceparts=/dev/null"),
        OsString::from("-o"),
        OsString::from("Dir::Etc::preferences=/dev/null"),
        OsString::from("-o"),
        OsString::from("Dir::Etc::preferencesparts=/dev/null"),
    ]
}

fn build_apt_command<S>(workdir: &Path, apt_opts: &[OsString], args: &[S]) -> Command
where
    S: AsRef<std::ffi::OsStr>,
{
    let mut cmd = Command::new("apt-get");
    cmd.env("APT_CONFIG", workdir.join("apt.conf"));
    cmd.args(apt_opts);
    cmd.args(args);
    cmd
}

fn run_command(mut cmd: Command, label: &str) -> Result<()> {
    let status = cmd.status()?;
    if status.success() {
        return Ok(());
    }
    Err(Error::Message(format!("{label} が失敗しました: {status}")))
}

fn collect_deb_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut debs = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("deb"))
        {
            debs.push(path);
        }
    }
    debs.sort();
    Ok(debs)
}

fn ensure_usrmerge_symlinks(root: &Path) -> Result<()> {
    create_usrmerge_symlink(root, "bin", "usr/bin")?;
    create_usrmerge_symlink(root, "sbin", "usr/sbin")?;
    create_usrmerge_symlink(root, "lib", "usr/lib")?;
    create_usrmerge_symlink(root, "lib64", "usr/lib64")?;
    Ok(())
}

fn create_usrmerge_symlink(root: &Path, legacy: &str, merged: &str) -> Result<()> {
    let legacy_path = root.join(legacy);
    let merged_path = root.join(merged);

    if legacy_path.symlink_metadata().is_ok() {
        return Ok(());
    }
    if !merged_path.is_dir() {
        return Ok(());
    }

    create_symlink(Path::new(merged), &legacy_path)
}

fn fix_absolute_symlinks(root: &Path) -> Result<()> {
    let mut symlinks = Vec::new();
    collect_symlinks(root, &mut symlinks)?;

    for link in symlinks {
        let target = fs::read_link(&link)?;
        if !target.is_absolute() {
            continue;
        }

        let inside = root.join(target.strip_prefix("/").unwrap_or(&target));
        if !inside.exists() {
            continue;
        }

        let link_parent = match link.parent() {
            Some(p) => p,
            None => continue,
        };
        let rel = relative_path(link_parent, &inside)?;

        fs::remove_file(&link)?;
        create_symlink(&rel, &link)?;
    }

    Ok(())
}

fn collect_symlinks(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            out.push(path);
        } else if file_type.is_dir() {
            collect_symlinks(&path, out)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn create_symlink(src: &Path, dst: &Path) -> Result<()> {
    std::os::unix::fs::symlink(src, dst)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_symlink(_src: &Path, _dst: &Path) -> Result<()> {
    Err(Error::Message(
        "このコマンドは現在 Unix 系 OS のみ対応です".to_string(),
    ))
}

fn relative_path(from: &Path, to: &Path) -> Result<PathBuf> {
    let from_abs = absolutize(from)?;
    let to_abs = absolutize(to)?;

    let from_comps: Vec<Component<'_>> = from_abs.components().collect();
    let to_comps: Vec<Component<'_>> = to_abs.components().collect();

    let mut common = 0usize;
    while common < from_comps.len()
        && common < to_comps.len()
        && from_comps[common] == to_comps[common]
    {
        common += 1;
    }

    let mut result = PathBuf::new();
    for comp in &from_comps[common..] {
        if matches!(comp, Component::Normal(_)) {
            result.push("..");
        }
    }
    for comp in &to_comps[common..] {
        result.push(comp.as_os_str());
    }

    if result.as_os_str().is_empty() {
        result.push(".");
    }

    Ok(result)
}

fn absolutize(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(path))
}

fn update_cargo_config(
    cwd: &Path,
    sysroot_dir: &Path,
    rust_target: &str,
    linker: &str,
    arch: &str,
) -> Result<()> {
    let cargo_dir = cwd.join(".cargo");
    fs::create_dir_all(&cargo_dir)?;
    let wrapper_paths = create_toolchain_wrappers(sysroot_dir, linker, arch)?;

    let config_path = cargo_dir.join("config.toml");
    let current = if config_path.exists() {
        fs::read_to_string(&config_path)?
    } else {
        String::new()
    };

    let rel_sysroot = relative_path(cwd, sysroot_dir)?;
    let rel_sysroot = rel_sysroot.to_string_lossy().to_string();
    let sysroot_arg = format!("link-arg=--sysroot={rel_sysroot}");
    let updated = upsert_target_section(&current, rust_target, linker, &sysroot_arg);
    let cc_value = relative_path(&cargo_dir, &wrapper_paths.cc)?;
    let cxx_value = relative_path(&cargo_dir, &wrapper_paths.cxx)?;
    let updated = upsert_env_section(
        &updated,
        rust_target,
        &cc_value.to_string_lossy(),
        &cxx_value.to_string_lossy(),
    );
    atomic_write(&config_path, &updated)
}

#[derive(Debug)]
struct WrapperPaths {
    cc: PathBuf,
    cxx: PathBuf,
}

fn create_toolchain_wrappers(sysroot_dir: &Path, linker: &str, arch: &str) -> Result<WrapperPaths> {
    let bundle_dir = sysroot_dir.parent().ok_or_else(|| {
        Error::Message(format!(
            "sysroot の親ディレクトリが取得できません: {}",
            sysroot_dir.display()
        ))
    })?;
    let wrapper_dir = bundle_dir.join("bin");
    fs::create_dir_all(&wrapper_dir)?;

    let rel_target_from_script = relative_path(&wrapper_dir, bundle_dir)?;
    let rel_target_from_script = rel_target_from_script.to_string_lossy();
    let script_common = format!(
        "SCRIPT_DIR=\"$(cd \"$(dirname \"$0\")\" && pwd)\"\nTARGET_DIR=\"$(cd \"$SCRIPT_DIR/{rel_target}\" && pwd)\"\nSYSROOT=\"$TARGET_DIR/sysroot\"\n",
        rel_target = rel_target_from_script
    );

    let cxx = infer_cxx_compiler(linker);
    let script_stem = wrapper_script_stem(linker);
    let cxx_script_stem = wrapper_script_stem(&cxx);
    let include_subdir = infer_include_subdir(linker, arch);
    let include_dir = format!("$SYSROOT/usr/include/{include_subdir}");

    let cc_path = wrapper_dir.join(format!("{script_stem}-with-sysroot.sh"));
    let cxx_path = wrapper_dir.join(format!("{cxx_script_stem}-with-sysroot.sh"));

    let cc_script = format!(
        "#!/usr/bin/env bash\nset -eu\n{script_common}\nexec {cc} --sysroot=\"$SYSROOT\" -isystem \"{include_dir}\" -isystem \"$SYSROOT/usr/include\" \"$@\"\n",
        cc = linker,
        include_dir = include_dir
    );
    let cxx_script = format!(
        "#!/usr/bin/env bash\nset -eu\n{script_common}\nexec {cxx} --sysroot=\"$SYSROOT\" \"$@\"\n",
        cxx = cxx
    );

    atomic_write(&cc_path, &cc_script)?;
    atomic_write(&cxx_path, &cxx_script)?;
    set_executable(&cc_path)?;
    set_executable(&cxx_path)?;

    let cc_abs = absolutize(&cc_path)?;
    let cxx_abs = absolutize(&cxx_path)?;
    Ok(WrapperPaths {
        cc: cc_abs,
        cxx: cxx_abs,
    })
}

fn infer_cxx_compiler(linker: &str) -> String {
    if let Some(prefix) = linker.strip_suffix("gcc") {
        return format!("{prefix}g++");
    }
    if let Some(prefix) = linker.strip_suffix("clang") {
        return format!("{prefix}clang++");
    }
    linker.to_string()
}

fn infer_include_subdir(linker: &str, arch: &str) -> String {
    if let Some(prefix) = linker.strip_suffix("-gcc")
        && !prefix.is_empty()
    {
        return prefix.to_string();
    }
    arch.to_string()
}

fn wrapper_script_stem(compiler: &str) -> String {
    Path::new(compiler)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(compiler)
        .to_string()
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn upsert_target_section(
    input: &str,
    rust_target: &str,
    linker: &str,
    sysroot_arg: &str,
) -> String {
    let mut lines: Vec<String> = input.lines().map(ToOwned::to_owned).collect();

    let section_name = format!("target.{rust_target}");
    let section_name = section_name.as_str();
    let section_header = format!("[{section_name}]");

    let target_start = lines
        .iter()
        .enumerate()
        .find_map(|(i, line)| (parse_section_name(line) == Some(section_name)).then_some(i));

    let linker_line = format!("linker = \"{}\"", toml_escape_basic_string(linker));
    let rustflags_line = format!(
        "rustflags = [\"-C\", \"{}\"]",
        toml_escape_basic_string(sysroot_arg)
    );

    match target_start {
        Some(start) => {
            let end = lines
                .iter()
                .enumerate()
                .skip(start + 1)
                .find_map(|(i, line)| parse_section_name(line).map(|_| i))
                .unwrap_or(lines.len());

            let replacement_lines = vec![linker_line, rustflags_line];
            let section_lines = rewrite_section_body(
                &lines,
                start,
                end,
                |line| is_key_line(line, "linker") || is_key_line(line, "rustflags"),
                &replacement_lines,
            );

            let mut merged = Vec::new();
            merged.extend_from_slice(&lines[..=start]);
            merged.extend(section_lines);
            merged.extend_from_slice(&lines[end..]);
            lines = merged;
        }
        None => {
            if !lines.is_empty() && !lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(section_header);
            lines.push(linker_line);
            lines.push(rustflags_line);
        }
    }

    let mut output = lines.join("\n");
    output.push('\n');
    output
}

fn upsert_env_section(input: &str, rust_target: &str, cc_value: &str, cxx_value: &str) -> String {
    let mut lines: Vec<String> = input.lines().map(ToOwned::to_owned).collect();

    let section_name = "env";
    let section_header = format!("[{section_name}]");
    let env_start = lines
        .iter()
        .enumerate()
        .find_map(|(i, line)| (parse_section_name(line) == Some(section_name)).then_some(i));

    let target_key = rust_target.replace('-', "_");
    let cc_key = format!("CC_{target_key}");
    let cxx_key = format!("CXX_{target_key}");
    let cflags_key = format!("CFLAGS_{target_key}");
    let cxxflags_key = format!("CXXFLAGS_{target_key}");

    let cc_line = format!(
        "{} = {{ value = \"{}\", relative = true }}",
        cc_key,
        toml_escape_basic_string(cc_value)
    );
    let cxx_line = format!(
        "{} = {{ value = \"{}\", relative = true }}",
        cxx_key,
        toml_escape_basic_string(cxx_value)
    );

    match env_start {
        Some(start) => {
            let end = lines
                .iter()
                .enumerate()
                .skip(start + 1)
                .find_map(|(i, line)| parse_section_name(line).map(|_| i))
                .unwrap_or(lines.len());

            let replacement_lines = vec![cc_line, cxx_line];
            let section_lines = rewrite_section_body(
                &lines,
                start,
                end,
                |line| {
                    is_key_line(line, &cc_key)
                        || is_key_line(line, &cxx_key)
                        || is_key_line(line, &cflags_key)
                        || is_key_line(line, &cxxflags_key)
                },
                &replacement_lines,
            );

            let mut merged = Vec::new();
            merged.extend_from_slice(&lines[..=start]);
            merged.extend(section_lines);
            merged.extend_from_slice(&lines[end..]);
            lines = merged;
        }
        None => {
            if !lines.is_empty() && !lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(section_header);
            lines.push(cc_line);
            lines.push(cxx_line);
        }
    }

    let mut output = lines.join("\n");
    output.push('\n');
    output
}

fn toml_escape_basic_string(value: &str) -> String {
    let value = value.replace('\\', "\\\\");
    value.replace('"', "\\\"")
}

fn rewrite_section_body<F>(
    lines: &[String],
    start: usize,
    end: usize,
    mut should_replace: F,
    replacements: &[String],
) -> Vec<String>
where
    F: FnMut(&str) -> bool,
{
    let mut out = Vec::new();
    let mut insert_at: Option<usize> = None;
    let mut i = start + 1;
    while i < end {
        let line = &lines[i];
        if should_replace(line) {
            if insert_at.is_none() {
                insert_at = Some(out.len());
            }
            i = skip_key_value_block(lines, i, end);
            continue;
        }
        out.push(line.clone());
        i += 1;
    }

    let at = insert_at.unwrap_or(out.len());
    out.splice(at..at, replacements.iter().cloned());
    out
}

fn skip_key_value_block(lines: &[String], start: usize, end: usize) -> usize {
    let line = &lines[start];
    let Some((_, rhs)) = line.split_once('=') else {
        return start + 1;
    };

    let mut depth = update_array_depth(0, rhs);
    let mut i = start + 1;
    while i < end && depth > 0 {
        depth = update_array_depth(depth, &lines[i]);
        i += 1;
    }
    i
}

fn update_array_depth(mut depth: i32, text: &str) -> i32 {
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escaped = false;

    for ch in text.chars() {
        if in_basic {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                in_basic = false;
            }
            continue;
        }

        if in_literal {
            if ch == '\'' {
                in_literal = false;
            }
            continue;
        }

        match ch {
            '"' => in_basic = true,
            '\'' => in_literal = true,
            '#' => break,
            '[' => depth += 1,
            ']' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ => {}
        }
    }

    depth
}

fn parse_section_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') || trimmed.starts_with("[[") {
        return None;
    }
    Some(&trimmed[1..trimmed.len() - 1])
}

fn is_key_line(line: &str, key: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return false;
    }
    let Some((lhs, _)) = trimmed.split_once('=') else {
        return false;
    };
    lhs.trim() == key
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        Error::Message(format!(
            "親ディレクトリが取得できません: {}",
            path.display()
        ))
    })?;
    let tmp_path = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("config.toml"),
        std::process::id()
    ));

    {
        let mut file = File::create(&tmp_path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }

    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample_config_json() -> &'static str {
        r#"{
  "name": "ubuntu-24.04_armv8",
  "arch": "arm64",
  "rust_target": "aarch64-unknown-linux-gnu",
  "linker": "aarch64-linux-gnu-gcc",
  "packages": ["libc6-dev", "libstdc++-13-dev"],
  "repos": [
    {
      "url": "http://ports.ubuntu.com/ubuntu-ports",
      "suites": ["noble", "noble-updates", "noble-security"],
      "components": ["main", "universe"]
    }
  ]
}"#
    }

    #[test]
    fn parse_args_require_config() {
        let argv = vec!["cargo-shiguredo-sysroot".to_string()];
        assert!(parse_args_from_argv(argv).is_err());
    }

    #[test]
    fn parse_args_accept_config() {
        let argv = vec![
            "cargo-shiguredo-sysroot".to_string(),
            "--config".to_string(),
            "ubuntu-24.04_armv8.json".to_string(),
        ];
        let args = parse_args_from_argv(argv).expect("parse args");
        assert_eq!(args.config_path, PathBuf::from("ubuntu-24.04_armv8.json"));
    }

    #[test]
    fn parse_sysroot_config_ok() {
        let config = parse_sysroot_config_text(sample_config_json()).expect("parse config");
        assert_eq!(config.name, "ubuntu-24.04_armv8");
        assert_eq!(config.arch, "arm64");
        assert_eq!(config.rust_target, "aarch64-unknown-linux-gnu");
        assert_eq!(config.linker, "aarch64-linux-gnu-gcc");
        assert_eq!(config.packages, vec!["libc6-dev", "libstdc++-13-dev"]);
        assert_eq!(config.repos.len(), 1);
        let repo = &config.repos[0];
        assert_eq!(repo.url, "http://ports.ubuntu.com/ubuntu-ports");
        assert_eq!(
            repo.suites,
            vec!["noble", "noble-updates", "noble-security"]
        );
        assert_eq!(repo.components, vec!["main", "universe"]);
    }

    #[test]
    fn parse_sysroot_config_invalid_name() {
        let config = sample_config_json().replace("\"ubuntu-24.04_armv8\"", "\"ubuntu/24.04\"");
        assert!(parse_sysroot_config_text(&config).is_err());
    }

    #[test]
    fn parse_sysroot_config_empty_packages() {
        let config = r#"{
  "name": "ubuntu-24.04_armv8",
  "arch": "arm64",
  "rust_target": "aarch64-unknown-linux-gnu",
  "linker": "aarch64-linux-gnu-gcc",
  "packages": [],
  "repos": [
    {
      "url": "http://ports.ubuntu.com/ubuntu-ports",
      "suites": ["noble"],
      "components": ["main"]
    }
  ]
}"#;
        assert!(parse_sysroot_config_text(config).is_err());
    }

    #[test]
    fn parse_sysroot_config_empty_repos() {
        let config = r#"{
  "name": "ubuntu-24.04_armv8",
  "arch": "arm64",
  "rust_target": "aarch64-unknown-linux-gnu",
  "linker": "aarch64-linux-gnu-gcc",
  "packages": ["libc6-dev"],
  "repos": []
}"#;
        assert!(parse_sysroot_config_text(config).is_err());
    }

    #[test]
    fn parse_sysroot_config_missing_repo_components() {
        let config = r#"{
  "name": "ubuntu-24.04_armv8",
  "arch": "arm64",
  "rust_target": "aarch64-unknown-linux-gnu",
  "linker": "aarch64-linux-gnu-gcc",
  "packages": ["libc6-dev"],
  "repos": [
    {
      "url": "http://ports.ubuntu.com/ubuntu-ports",
      "suites": ["noble"]
    }
  ]
}"#;
        assert!(parse_sysroot_config_text(config).is_err());
    }

    #[test]
    fn parse_sysroot_config_accept_unknown_arch_string() {
        let config = sample_config_json().replace("\"arm64\"", "\"x86_64\"");
        let config = parse_sysroot_config_text(&config).expect("parse config");
        assert_eq!(config.arch, "x86_64");
    }

    #[test]
    fn upsert_target_section_create() {
        let output = upsert_target_section(
            "",
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
        );
        assert!(output.contains("[target.aarch64-unknown-linux-gnu]"));
        assert!(output.contains("linker = \"aarch64-linux-gnu-gcc\""));
        assert!(output.contains(
            "rustflags = [\"-C\", \"link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot\"]"
        ));
    }

    #[test]
    fn upsert_target_section_update_keep_other_keys() {
        let input = r#"[target.aarch64-unknown-linux-gnu]
linker = "old"
rustflags = ["-C", "old"]
foo = "bar"

[env]
A = "B"
"#;
        let output = upsert_target_section(
            input,
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
        );
        assert!(output.contains("foo = \"bar\""));
        assert!(output.contains("[env]"));
        assert!(!output.contains("linker = \"old\""));
        assert!(!output.contains("\"old\""));
    }

    #[test]
    fn upsert_target_section_update_multiline_rustflags() {
        let input = r#"[target.aarch64-unknown-linux-gnu]
rustflags = [
  "-C",
  "link-arg=--sysroot=old",
]
foo = "bar"
"#;
        let output = upsert_target_section(
            input,
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
        );
        assert!(output.contains("foo = \"bar\""));
        assert!(output.contains(
            "rustflags = [\"-C\", \"link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot\"]"
        ));
        assert!(!output.contains("link-arg=--sysroot=old"));
        assert!(!output.contains("  \"-C\","));
    }

    #[test]
    fn upsert_target_section_preserve_expected_blank_lines() {
        let input = r#"[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
rustflags = ["-C", "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot"]

[env]
"#;
        let output = upsert_target_section(
            input,
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
        );
        assert!(output.contains(
            "[target.aarch64-unknown-linux-gnu]\nlinker = \"aarch64-linux-gnu-gcc\"\nrustflags = [\"-C\", \"link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot\"]\n\n[env]\n"
        ));
        assert!(!output.contains("[target.aarch64-unknown-linux-gnu]\n\nlinker"));
    }

    #[test]
    fn upsert_target_then_env_section_keep_multiline_input_consistent() {
        let input = r#"[target.aarch64-unknown-linux-gnu]
rustflags = [
  "-C",
  "link-arg=--sysroot=old",
]

[env]
FOO = "BAR"
"#;
        let updated = upsert_target_section(
            input,
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
        );
        let updated = upsert_env_section(
            &updated,
            "aarch64-unknown-linux-gnu",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        );
        assert!(updated.contains("[target.aarch64-unknown-linux-gnu]"));
        assert!(updated.contains("[env]"));
        assert!(updated.contains("FOO = \"BAR\""));
        assert!(updated.contains(
            "rustflags = [\"-C\", \"link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot\"]"
        ));
        assert!(!updated.contains("link-arg=--sysroot=old"));
        assert_eq!(updated.matches("rustflags = ").count(), 1);
    }

    #[test]
    fn upsert_target_section_idempotent() {
        let once = upsert_target_section(
            "",
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
        );
        let twice = upsert_target_section(
            &once,
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
        );
        assert_eq!(once, twice);
    }

    #[test]
    fn upsert_env_section_create() {
        let output = upsert_env_section(
            "",
            "aarch64-unknown-linux-gnu",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        );
        assert!(output.contains("[env]"));
        assert!(
            output.contains("CC_aarch64_unknown_linux_gnu = { value = \"../target/cc-wrapper.sh\", relative = true }")
        );
        assert!(
            output.contains("CXX_aarch64_unknown_linux_gnu = { value = \"../target/cxx-wrapper.sh\", relative = true }")
        );
    }

    #[test]
    fn upsert_env_section_update_keep_other_keys() {
        let input = r#"[env]
FOO = "BAR"
CC_aarch64_unknown_linux_gnu = "old-cc"
CFLAGS_aarch64_unknown_linux_gnu = "old-cflags"

[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
"#;
        let output = upsert_env_section(
            input,
            "aarch64-unknown-linux-gnu",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        );
        assert!(output.contains("FOO = \"BAR\""));
        assert!(output.contains("[target.aarch64-unknown-linux-gnu]"));
        assert!(!output.contains("old-cc"));
        assert!(!output.contains("old-cflags"));
        assert!(output.contains("CC_aarch64_unknown_linux_gnu = { value = \"../target/cc-wrapper.sh\", relative = true }"));
        assert!(output.contains("CXX_aarch64_unknown_linux_gnu = { value = \"../target/cxx-wrapper.sh\", relative = true }"));
    }

    #[test]
    fn upsert_env_section_idempotent() {
        let once = upsert_env_section(
            "",
            "aarch64-unknown-linux-gnu",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        );
        let twice = upsert_env_section(
            &once,
            "aarch64-unknown-linux-gnu",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        );
        assert_eq!(once, twice);
    }

    #[test]
    fn build_apt_options_ignore_host_preferences() {
        let opts = build_apt_options(Path::new("/tmp/sysroot-work"));
        let joined = opts
            .iter()
            .map(|v| v.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("Dir::Etc::preferences=/dev/null"));
        assert!(joined.contains("Dir::Etc::preferencesparts=/dev/null"));
    }

    #[test]
    fn update_cargo_config_creates_wrapper_scripts_under_target() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("duration")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("shiguredo-sysroot-test-{unique}"));
        let sysroot = root.join("target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot");
        fs::create_dir_all(&sysroot).expect("create sysroot dir");

        update_cargo_config(
            &root,
            &sysroot,
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "arm64",
        )
        .expect("update config");

        let config = fs::read_to_string(root.join(".cargo/config.toml")).expect("read config");
        assert!(config.contains("[target.aarch64-unknown-linux-gnu]"));

        let rel_sysroot = relative_path(&root, &sysroot).expect("relative sysroot path");
        let sysroot_arg = format!("link-arg=--sysroot={}", rel_sysroot.to_string_lossy());
        let rustflags_line = format!(
            "rustflags = [\"-C\", \"{}\"]",
            toml_escape_basic_string(&sysroot_arg)
        );
        assert!(config.contains(&rustflags_line));

        let cc_wrapper = root.join(
            "target/shiguredo-sysroot/ubuntu-24.04_armv8/bin/aarch64-linux-gnu-gcc-with-sysroot.sh",
        );
        let cxx_wrapper = root.join(
            "target/shiguredo-sysroot/ubuntu-24.04_armv8/bin/aarch64-linux-gnu-g++-with-sysroot.sh",
        );
        assert!(cc_wrapper.exists());
        assert!(cxx_wrapper.exists());

        let cargo_dir = root.join(".cargo");
        let cc_rel = relative_path(&cargo_dir, &cc_wrapper).expect("relative cc wrapper path");
        let cxx_rel = relative_path(&cargo_dir, &cxx_wrapper).expect("relative cxx wrapper path");
        let cc_line = format!(
            "CC_aarch64_unknown_linux_gnu = {{ value = \"{}\", relative = true }}",
            toml_escape_basic_string(&cc_rel.to_string_lossy())
        );
        let cxx_line = format!(
            "CXX_aarch64_unknown_linux_gnu = {{ value = \"{}\", relative = true }}",
            toml_escape_basic_string(&cxx_rel.to_string_lossy())
        );
        assert!(config.contains(&cc_line));
        assert!(config.contains(&cxx_line));

        let cc_script = fs::read_to_string(&cc_wrapper).expect("read cc wrapper");
        assert!(cc_script.contains("exec aarch64-linux-gnu-gcc --sysroot=\"$SYSROOT\""));
        assert!(cc_script.contains("-isystem \"$SYSROOT/usr/include/aarch64-linux-gnu\""));

        let cxx_script = fs::read_to_string(&cxx_wrapper).expect("read cxx wrapper");
        assert!(cxx_script.contains("exec aarch64-linux-gnu-g++ --sysroot=\"$SYSROOT\""));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&cc_wrapper)
                .expect("cc metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn normalize_argv_for_noargs_strip_cargo_subcommand() {
        let argv = vec![
            "cargo-shiguredo-sysroot".to_string(),
            "shiguredo-sysroot".to_string(),
            "--config".to_string(),
            "ubuntu-24.04_armv8.json".to_string(),
        ];
        let normalized = normalize_argv_for_noargs(argv);
        assert_eq!(
            normalized,
            vec![
                "cargo-shiguredo-sysroot".to_string(),
                "--config".to_string(),
                "ubuntu-24.04_armv8.json".to_string()
            ]
        );
    }

    #[test]
    fn normalize_argv_for_noargs_keep_direct_invocation() {
        let argv = vec![
            "cargo-shiguredo-sysroot".to_string(),
            "--config".to_string(),
            "ubuntu-24.04_armv8.json".to_string(),
        ];
        let normalized = normalize_argv_for_noargs(argv.clone());
        assert_eq!(normalized, argv);
    }
}
