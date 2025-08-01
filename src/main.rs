mod dol;
mod gui;
mod hex_literal;
mod iso_tools;
mod patch_loader;
mod patcher;
mod paths;

use anyhow::Context;
use clap::Parser;
use dialoguer::Confirm;
use indicatif::ProgressBar;
use iso_tools::*;
use rfd::FileDialog;
use std::{env, fs, time::Duration};
use semver::Version;
use self_update::{self, backends::github::Update, cargo_crate_version};

pub const CURRENT_VERSION: &str = cargo_crate_version!();

const REPO_OWNER: &str = "calebh13";
const REPO_NAME: &str = "ssgz";    
const BIN_NAME: &str = "ssgz";

#[derive(Parser, Debug)]
#[clap(about = "Practice ROM Hack Patcher for Skyward Sword")]
#[clap(version = CURRENT_VERSION)]
struct Args {
    #[arg(long)]
    noui: bool,
    #[arg(requires = "noui")]
    game_version: Option<GameVersion>,
}

fn current_platform_suffix() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "macos_intel"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "macos_apple_silicon"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

fn check_for_updates(args: &Args) -> anyhow::Result<()> {
    let target_name = format!("SSGZ.{CURRENT_VERSION}.{}.zip", current_platform_suffix());

    let update = self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME) // or "ssgz.exe" on Windows, if needed
        .show_download_progress(true)
        .current_version(CURRENT_VERSION)
        .target(&target_name)
        .build()?;


    let release = update
        .get_latest_release()
        .context("Failed to fetch latest GitHub release")?;

    let latest_version = Version::parse(&release.version)
        .context("Failed to parse latest version from GitHub")?;

    let current_version = Version::parse(CURRENT_VERSION)
        .context("Failed to parse current version")?;

    if latest_version <= current_version {
        println!("Already up to date: v{}", CURRENT_VERSION);
        return Ok(());
    }

    println!(
        "Update available: v{} → v{}",
        CURRENT_VERSION, latest_version.to_string()
    );

    // TODO: figure out how we want to handle this
    // let exe_path_str = env::current_exe()
    //     .context("Failed to get current executable path")?
    //     .to_string_lossy()
    //     .to_string();
    
    // if exe_path_str.contains("target/release") || exe_path_str.contains("target/debug") {
    //     println!("Running from source; skipping automatic update.");
    //     println!("Please update manually using git pull && cargo build");
    //     return Ok(());
    // }

    if !Confirm::new()
        .with_prompt("Do you want to update now?")
        .default(false)
        .interact()
        .context("Failed to read user input")?
    {
        println!("Update canceled.");
        return Ok(());
    }

    // If not running from source, download and show progress bar
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(Duration::from_millis(100));
    pb.set_message("Downloading update ...");

    let status = Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .show_output(false) // we'll show our own output
        .current_version(&CURRENT_VERSION.to_string())
        .build()
        .context("Failed to configure self-update for actual download")?
        .update()
        .context("Update failed")?;

    pb.finish_and_clear();

    println!("Updated successfully to v{}!", status.version());
    Ok(())
}


fn fix_macos_working_directory() -> anyhow::Result<()> {
    // If in a .app file, we need to fix working directory to the bundle's location
    // (unless running from source or with dx serve)
    #[cfg(target_os = "macos")]
    {
        if let Ok(exe_path) = env::current_exe() {
            let mut current = exe_path.as_path();
            while let Some(parent) = current.parent() {
                if parent.extension().map_or(false, |ext| ext == "app") {
                    let app_parent = parent.parent().unwrap_or(parent);
                    let app_parent_str = app_parent.to_string_lossy();

                    // Keep current directory if testing with dx serve
                    if app_parent_str.contains("/target/dx") {
                        println!(
                            "Development .app detected (in target directory), keeping current working directory"
                        );
                        return Ok(());
                    }

                    env::set_current_dir(app_parent)?;
                    println!(
                        "Distributed .app bundle detected, working directory set to: {:?}",
                        app_parent
                    );
                    return Ok(());
                }
                current = parent;
            }
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    fix_macos_working_directory()?;
    let args = Args::parse();
    check_for_updates(&args)?;
    if args.noui {
        if let Some(ver) = args.game_version {
            do_noui(ver)
        } else {
            panic!("When using --noui, you must specify a version.")
        }
    } else {
        gui::do_gui();
        Ok(())
    }
}

pub fn is_ready_to_patch(version: GameVersion) -> bool {
    paths::extract_dol_exists(version) && paths::dol_copy_exists(version)
}

fn do_noui(version: GameVersion) -> anyhow::Result<()> {
    assert!(version.is_supported()); // arg parser should only accept supported versions

    println!("Starting SSGZ Patcher {CURRENT_VERSION} for the {version} version");

    let extract_done = paths::extract_dol_exists(version);
    let dol_copied = paths::dol_copy_exists(version);

    if !(extract_done && dol_copied) {
        let ver_str = version.to_string();
        if !extract_done {
            println!(
                "Please provide a clean copy of the {ver_str} version to create a practice ROM."
            );
        } else {
            println!(
                "Couldn't find copy of original main.dol file. It is recommended to redo extraction for the {ver_str} version."
            );
        }

        do_extract_noui(version)?;
    }

    patcher::do_gz_patches(version)?;

    let repack_iso = Confirm::new()
        .with_prompt("Patching done, do you want to write an output iso?")
        .interact()
        .unwrap();

    if repack_iso {
        let bar = ProgressBar::new(100);
        bar.set_style(
            indicatif::ProgressStyle::with_template(
                "{spinner:.green} [{wide_bar:.cyan/blue}] {percent}% ({eta})",
            )
            .unwrap()
            .progress_chars("#>-"),
        );
        do_repack(version, &mut |done_percent| {
            bar.set_position(done_percent as u64);
        })?;
        bar.finish_with_message("Rebuilding done.");
    }

    println!(
        "All done, happy speedrunning! Press Z and C simultaneously to access practice menus!"
    );
    Ok(())
}

fn do_extract_noui(version: GameVersion) -> anyhow::Result<()> {
    let ver_str = version.to_string();
    let file = FileDialog::new()
        .set_title(format!("Select a clean {ver_str} ISO."))
        .add_filter("Game ISO", &["iso"])
        .set_directory("./")
        .pick_file()
        .with_context(|| "Must have chosen an iso file.")?;

    // Attempt to extract iso and validate the correct version was given
    let mut extractor = WiiIsoExtractor::new_with_version(file, version)?;
    let ext_path = paths::extract_path(version);
    fs::create_dir_all(&ext_path)?;
    let total_bytes = extractor.size_of_extract()? as u64;
    // Use indicatif's ProgressBar to display progress in the terminal
    println!("Extracting files...");
    let bar = ProgressBar::new(total_bytes as u64);
    bar.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.green} [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})",
        )
        .unwrap()
        .progress_chars("#>-"),
    );
    extractor.extract_to(ext_path.clone(), &mut |done_bytes| {
        bar.set_position(done_bytes);
    })?;
    bar.finish_with_message("Extraction done.");
    paths::copy_dol_after_extract(version)?;

    Ok(())
}

fn do_extract_ui<T: FnMut(u8)>(version: GameVersion, callback: &mut T) -> anyhow::Result<()> {
    let ver_str = version.to_string();
    let file = FileDialog::new()
        .set_title(format!("Select a clean {ver_str} ISO."))
        .add_filter("Game ISO", &["iso"])
        .set_directory("./")
        .pick_file()
        .with_context(|| "Must have chosen an iso file.")?;

    // Attempt to extract iso and validate the correct version was given
    let mut extractor = WiiIsoExtractor::new_with_version(file, version)?;
    let ext_path = paths::extract_path(version);
    fs::create_dir_all(&ext_path)?;
    let total_bytes = extractor.size_of_extract()? as u64;
    // Here, callback operates on the extraction percentage rather than raw byte count
    extractor.extract_to(ext_path.clone(), &mut |done_bytes| {
        callback(((done_bytes * 100) / total_bytes) as u8);
    })?;
    paths::copy_dol_after_extract(version)?;

    Ok(())
}

fn do_repack<T: FnMut(u8)>(version: GameVersion, callback: &mut T) -> anyhow::Result<()> {
    let mut out_dir = FileDialog::new()
        .set_title("Choose a directory for the patched ISO.")
        .set_directory("./")
        .pick_folder()
        .with_context(|| "Must have chosen an output directory.")?;

    out_dir.push(version.iso_name());

    rebuild_from_directory(paths::extract_path(version), out_dir, &mut |done_percent| {
        callback(done_percent);
    })?;

    Ok(())
}
