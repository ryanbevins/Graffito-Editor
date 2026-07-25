use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod gltf_fixtures;

const US_REGRESSION_FOLDER: &str = "SunshineUSExport";
const EXPECTED_US_STAGE_COUNT: usize = 108;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "could not resolve the workspace root".to_string())?;

    match arguments.first().and_then(|argument| argument.to_str()) {
        Some("regression") => run_regression(repo_root, RegressionOptions::parse(arguments)?),
        Some("gltf-fixtures") => gltf_fixtures::run(repo_root, &arguments[1..]),
        Some("schema-bundle") => {
            run_schema_bundle(repo_root, SchemaBundleOptions::parse(arguments)?)
        }
        Some(command) => Err(usage(&format!("unknown command '{command}'"))),
        None => Err(usage("missing command")),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SchemaBundleOptions {
    decomp_root: PathBuf,
    source_revision: Option<String>,
    check: bool,
}

impl SchemaBundleOptions {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut arguments = arguments.into_iter();
        let command = arguments.next().ok_or_else(|| usage("missing command"))?;
        if command != OsStr::new("schema-bundle") {
            return Err(usage("expected the schema-bundle command"));
        }

        let mut decomp_root = None;
        let mut source_revision = None;
        let mut check = false;
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--decomp-root") => {
                    let path = arguments
                        .next()
                        .ok_or_else(|| usage("--decomp-root requires a path"))?;
                    decomp_root = Some(PathBuf::from(path));
                }
                Some("--source-revision") => {
                    let revision = arguments
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(|| usage("--source-revision requires a Unicode value"))?;
                    if revision.trim().is_empty() {
                        return Err(usage("--source-revision cannot be empty"));
                    }
                    source_revision = Some(revision);
                }
                Some("--check") => check = true,
                Some("--help" | "-h") => return Err(usage("")),
                Some(other) => return Err(usage(&format!("unknown argument '{other}'"))),
                None => return Err(usage("arguments must be valid Unicode")),
            }
        }

        Ok(Self {
            decomp_root: decomp_root
                .ok_or_else(|| usage("--decomp-root is required for schema-bundle"))?,
            source_revision,
            check,
        })
    }
}

fn run_schema_bundle(repo_root: &Path, options: SchemaBundleOptions) -> Result<(), String> {
    let decomp_root = fs::canonicalize(&options.decomp_root).map_err(|error| {
        format!(
            "could not resolve decomp root {}: {error}",
            options.decomp_root.display()
        )
    })?;
    let source_revision = resolve_source_revision(&decomp_root, options.source_revision)?;
    let bundle = sms_schema::SchemaGenerator::new(&decomp_root)
        .generate_bundle(source_revision)
        .map_err(|error| format!("schema generation failed: {error}"))?;
    let bytes = bundle
        .to_pretty_vec()
        .map_err(|error| format!("could not serialize schema bundle: {error}"))?;
    let output_path = repo_root
        .join("crates")
        .join("sms-schema")
        .join("generated")
        .join("object-registry.json");

    if options.check {
        let committed = fs::read(&output_path).map_err(|error| {
            format!(
                "could not read bundled schema {}: {error}",
                output_path.display()
            )
        })?;
        if committed != bytes {
            return Err(format!(
                "bundled schema is stale; regenerate {} from decomp revision {}",
                output_path.display(),
                bundle.source_revision
            ));
        }
        println!(
            "Bundled schema matches decomp revision {} ({:#018x}).",
            bundle.source_revision, bundle.source_fingerprint
        );
        return Ok(());
    }

    fs::write(&output_path, bytes).map_err(|error| {
        format!(
            "could not write bundled schema {}: {error}",
            output_path.display()
        )
    })?;
    println!(
        "Wrote {} from decomp revision {} ({:#018x}); {} objects, {} NPC families, {} enemy \
         actors, {} music tracks, {} stage-audio areas, {} dialogue voices.",
        output_path.display(),
        bundle.source_revision,
        bundle.source_fingerprint,
        bundle.registry.objects.len(),
        bundle.registry.npc_actors.len(),
        bundle.registry.enemy_actors.len(),
        bundle.registry.bgm_wave_scenes.len(),
        bundle.registry.stage_audio_areas.len(),
        bundle.registry.dialogue_voices.len()
    );
    Ok(())
}

fn clean_git_revision(repo_root: &Path) -> Result<String, String> {
    let status = Command::new("git")
        .current_dir(repo_root)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .map_err(|error| format!("could not inspect decomp git status: {error}"))?;
    if !status.status.success() {
        return Err("could not inspect decomp git status".to_string());
    }
    if !status.stdout.is_empty() {
        return Err(
            "decomp checkout has tracked changes; generate from a clean checkout or a clean \
             exported source tree"
                .to_string(),
        );
    }

    let revision = Command::new("git")
        .current_dir(repo_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("could not resolve decomp revision: {error}"))?;
    if !revision.status.success() {
        return Err("could not resolve decomp revision".to_string());
    }
    let revision = String::from_utf8(revision.stdout)
        .map_err(|_| "decomp revision was not valid UTF-8".to_string())?;
    let revision = revision.trim().to_string();
    if revision.is_empty() {
        return Err("decomp revision was empty".to_string());
    }
    Ok(revision)
}

fn resolve_source_revision(
    repo_root: &Path,
    requested_revision: Option<String>,
) -> Result<String, String> {
    if repo_root.join(".git").exists() {
        let actual_revision = clean_git_revision(repo_root)?;
        if let Some(requested_revision) = requested_revision {
            if requested_revision != actual_revision {
                return Err(format!(
                    "requested source revision {requested_revision} does not match clean checkout \
                     revision {actual_revision}"
                ));
            }
        }
        return Ok(actual_revision);
    }

    requested_revision.ok_or_else(|| {
        "an exported decomp source tree requires an explicit --source-revision".to_string()
    })
}

fn run_regression(repo_root: &Path, options: RegressionOptions) -> Result<(), String> {
    gltf_fixtures::check(repo_root)?;
    run_cargo(repo_root, &["fmt", "--all", "--", "--check"], None)?;
    run_cargo(
        repo_root,
        &[
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        None,
    )?;
    run_cargo(repo_root, &["test", "--locked", "--workspace"], None)?;

    if !options.code_only {
        let base_root = select_retail_root(options.base_root)?;
        validate_retail_root(&base_root)?;
        println!("\n==> Source-free retail census: {}", base_root.display());
        run_cargo(
            repo_root,
            &[
                "test",
                "--locked",
                "-p",
                "sms-scene",
                "stage_archive::tests::source_free_rebuilds_every_retail_stage_archive",
                "--",
                "--ignored",
                "--exact",
                "--nocapture",
            ],
            Some((&base_root, "SMS_BASE_ROOT")),
        )?;
    }

    run_cargo(
        repo_root,
        &["build", "--locked", "--release", "-p", "graffito-editor"],
        None,
    )?;
    println!("\nAll requested regression gates passed.");
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct RegressionOptions {
    code_only: bool,
    base_root: Option<PathBuf>,
}

impl RegressionOptions {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut arguments = arguments.into_iter();
        let command = arguments.next().ok_or_else(|| usage("missing command"))?;
        if command != OsStr::new("regression") {
            return Err(usage("only the regression command is supported"));
        }

        let mut options = Self::default();
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--code-only") => options.code_only = true,
                Some("--base-root") => {
                    let path = arguments
                        .next()
                        .ok_or_else(|| usage("--base-root requires a path"))?;
                    options.base_root = Some(PathBuf::from(path));
                }
                Some("--help" | "-h") => return Err(usage("")),
                Some(other) => return Err(usage(&format!("unknown argument '{other}'"))),
                None => return Err(usage("arguments must be valid Unicode")),
            }
        }
        if options.code_only && options.base_root.is_some() {
            return Err(usage("--code-only cannot be combined with --base-root"));
        }
        Ok(options)
    }
}

fn usage(error: &str) -> String {
    let prefix = if error.is_empty() {
        String::new()
    } else {
        format!("{error}\n\n")
    };
    format!(
        "{prefix}usage:\n  cargo regression [--code-only | --base-root <EXTRACTED_US_ROOT>]\n  \
         cargo gltf-fixtures [--check]\n  \
         cargo schema-bundle --decomp-root <DECOMP_ROOT> [--source-revision <REVISION>] [--check]"
    )
}

fn select_retail_root(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    if let Some(user_profile) = env::var_os("USERPROFILE") {
        let preferred = PathBuf::from(user_profile)
            .join("Downloads")
            .join(US_REGRESSION_FOLDER);
        if preferred.is_dir() {
            return Ok(preferred);
        }
    }

    if let Some(path) = env::var_os("SMS_BASE_ROOT") {
        return Ok(PathBuf::from(path));
    }

    Err(format!(
        "no retail baseline found; expected %USERPROFILE%\\Downloads\\{US_REGRESSION_FOLDER}, \
         SMS_BASE_ROOT, or an explicit --base-root"
    ))
}

fn validate_retail_root(base_root: &Path) -> Result<(), String> {
    let scene_root = base_root.join("files").join("data").join("scene");
    if !scene_root.is_dir() {
        return Err(format!(
            "retail baseline has no files/data/scene directory: {}",
            base_root.display()
        ));
    }
    let stage_count = fs::read_dir(&scene_root)
        .map_err(|error| format!("could not read {}: {error}", scene_root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("szs"))
        })
        .count();
    if stage_count != EXPECTED_US_STAGE_COUNT {
        return Err(format!(
            "expected {EXPECTED_US_STAGE_COUNT} US .szs stages in {}, found {stage_count}",
            scene_root.display()
        ));
    }
    if !scene_root.join("test11.szs").is_file() {
        return Err(format!(
            "US retail baseline is missing {}",
            scene_root.join("test11.szs").display()
        ));
    }
    Ok(())
}

fn run_cargo(
    repo_root: &Path,
    arguments: &[&str],
    environment: Option<(&Path, &str)>,
) -> Result<(), String> {
    println!("\n==> cargo {}", arguments.join(" "));
    let mut command = Command::new("cargo");
    command.current_dir(repo_root).args(arguments);
    if let Some((value, name)) = environment {
        command.env(name, value);
    }
    let status = command
        .status()
        .map_err(|error| format!("could not run cargo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "cargo {} exited with {status}",
            arguments.join(" ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_full_regression() {
        assert_eq!(
            RegressionOptions::parse([OsString::from("regression")]).unwrap(),
            RegressionOptions::default()
        );
    }

    #[test]
    fn parses_explicit_root_and_code_only_modes() {
        assert_eq!(
            RegressionOptions::parse([
                OsString::from("regression"),
                OsString::from("--base-root"),
                OsString::from("C:/retail"),
            ])
            .unwrap(),
            RegressionOptions {
                code_only: false,
                base_root: Some(PathBuf::from("C:/retail")),
            }
        );
        assert_eq!(
            RegressionOptions::parse(
                [OsString::from("regression"), OsString::from("--code-only"),]
            )
            .unwrap(),
            RegressionOptions {
                code_only: true,
                base_root: None,
            }
        );
    }

    #[test]
    fn rejects_conflicting_modes() {
        let error = RegressionOptions::parse([
            OsString::from("regression"),
            OsString::from("--code-only"),
            OsString::from("--base-root"),
            OsString::from("C:/retail"),
        ])
        .unwrap_err();
        assert!(error.contains("cannot be combined"));
    }

    #[test]
    fn parses_schema_bundle_options() {
        assert_eq!(
            SchemaBundleOptions::parse([
                OsString::from("schema-bundle"),
                OsString::from("--decomp-root"),
                OsString::from("../decomp"),
                OsString::from("--source-revision"),
                OsString::from("abc123"),
                OsString::from("--check"),
            ])
            .unwrap(),
            SchemaBundleOptions {
                decomp_root: PathBuf::from("../decomp"),
                source_revision: Some("abc123".to_string()),
                check: true,
            }
        );
    }

    #[test]
    fn schema_bundle_requires_decomp_root() {
        let error = SchemaBundleOptions::parse([OsString::from("schema-bundle")]).unwrap_err();
        assert!(error.contains("--decomp-root is required"));
    }
}
