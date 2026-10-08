//! `bluemap`: drop-in for `java -jar bluemap-cli.jar` (`BlueMapCLI.main`): same options, config folder handling and
//! exit codes (1 configuration/IO error, 2 missing resources).

#[cfg(target_env = "musl")]
mod alloc;
mod args;
mod convert;
mod eta;
mod log;
mod plugin;
mod render;
mod shutdown;
mod watch;
mod web;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use args::{Args, HELP};
use bm_config::{BlueMapConfig, ConfigOptions};
use bm_engine::{ResourceOptions, Service, TileUpdateStrategy};
use clap::Parser;
use shutdown::Shutdown;

/// bluemap-rs version: a release build's (`BLUEMAP_RS_VERSION` from `tools/build_core.py`), else Cargo's.
pub const VERSION: &str = match option_env!("BLUEMAP_RS_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

fn main() -> ExitCode {
    #[cfg(target_env = "musl")]
    alloc::tune();
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            log::error(&format!("Failed to parse provided arguments! {}", e.to_string().trim()));
            print!("{HELP}");
            return ExitCode::from(1);
        }
    };
    if args.plugin_ipc {
        return plugin::run(args.parent_pid);
    }
    match run(&args) {
        Ok(code) => code,
        Err(e) if is_missing_resources(&e) => {
            log::warn("BlueMap is missing important resources!");
            log::warn("You must accept the required file download in order for BlueMap to work!");
            let core = bm_config::resolve_config_file(&config_folder(&args), "core");
            log::warn(&format!("Please check: {}", absolute(&core).display()));
            ExitCode::from(2)
        }
        Err(e) => {
            log::error(&format!("{e:#}"));
            ExitCode::from(1)
        }
    }
}

fn is_missing_resources(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<bm_engine::Error>(), Some(bm_engine::Error::MissingResources(_)))
}

fn config_folder(args: &Args) -> PathBuf {
    args.config.clone().unwrap_or_else(|| PathBuf::from("config"))
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_owned())
}

fn run(args: &Args) -> Result<ExitCode> {
    if let Some(file) = &args.log_file {
        let file = file.to_string_lossy();
        log::add_file(&file, args.append).with_context(|| format!("log file {file}"))?;
    }
    if args.help {
        print!("{HELP}");
        return Ok(ExitCode::SUCCESS);
    }
    if args.version {
        println!("{}\nbluemap-rs {VERSION}", bm_engine::BLUEMAP_VERSION);
        return Ok(ExitCode::SUCCESS);
    }
    let config_folder = config_folder(args);
    std::fs::create_dir_all(&config_folder).with_context(|| format!("create {}", config_folder.display()))?;
    if let Some(mods) = &args.mods
        && !mods.is_dir()
    {
        bail!("Mods folder does not exist: {}", mods.display());
    }
    let packs = config_folder.join("packs");
    std::fs::create_dir_all(&packs).with_context(|| format!("create {}", packs.display()))?;
    bm_engine::find_java_addons(&packs).iter().for_each(|a| log::warn(&a.warning()));

    let config = BlueMapConfig::load(&ConfigOptions::cli(&config_folder))?;
    if let Some(file) = &config.core.log.file {
        log::add_formatted_file(file, config.core.log.append).with_context(|| format!("log file {file}"))?;
    }
    let threads = config.core.resolve_render_thread_count(std::thread::available_parallelism().map_or(1, |n| n.get()));
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("bluemap-render-{i}"))
        .build_global()
        .context("start render threads")?;
    let options = ResourceOptions {
        minecraft_version: args.mc_version.clone(),
        packs_folder: Some(packs),
        mods_folder: args.mods.clone(),
    };
    let service = Service::new(config, options);
    if let (Some(id), Some(to)) = (&args.convert_storage, &args.to) {
        convert::convert_storage(&service, &config_folder, id, to)?;
        return Ok(ExitCode::SUCCESS);
    }
    let shutdown = Shutdown::install();

    let mut ok = true;
    let mut webserver = None;
    if args.renders() {
        let (mut maps, failed) = render::load_maps(&service, args.maps.as_deref(), args.generate_webapp)?;
        if args.webserver {
            webserver = Some(web::start(&service, args.verbose, &mut maps, &shutdown)?);
        }
        let strategy = if args.force_render {
            TileUpdateStrategy::ForceAll
        } else if args.fix_edges {
            TileUpdateStrategy::ForceEdge
        } else {
            TileUpdateStrategy::ForceNone
        };
        let web = webserver.as_ref().map(web::Webserver::maps);
        ok = render::run(&service, maps, failed, strategy, args.watch, web, &shutdown)?;
    } else {
        if args.markers {
            update_markers(&service, args.maps.as_deref());
        }
        if args.generate_webapp {
            bm_web::install_webapp(&service.config.webapp.webroot, true)?;
            service.write_webapp_settings()?;
        } else if args.generate_websettings {
            service.write_webapp_settings()?;
        }
        if args.webserver {
            webserver = Some(web::start(&service, args.verbose, &mut [], &shutdown)?);
        }
    }
    if let Some(server) = webserver {
        server.wait()?;
    }
    if args.renders() || args.webserver || args.generate_webapp || args.generate_websettings || args.markers {
        return Ok(if ok { ExitCode::SUCCESS } else { ExitCode::from(1) });
    }
    log::info(&format!("Generated default config files for you, here: {}\n", absolute(&config_folder).display()));
    print!("{HELP}");
    Ok(ExitCode::from(1))
}

/// `BlueMapCLI.updateMarkers`; unlike Java, a map outside `-m` doesn't end the loop early.
fn update_markers(service: &Service, maps: Option<&str>) {
    let selected: Option<Vec<&str>> = maps.map(|m| m.split(',').collect());
    for id in service.map_ids(|id| selected.as_ref().is_none_or(|s| s.contains(&id))) {
        match service.write_config_markers(&id) {
            Ok(warnings) => {
                warnings.iter().for_each(|w| log::warn(w));
                log::info(&format!("Updated markers for map '{id}'"));
            }
            Err(e) => log::error(&format!("Failed to save markers for map '{id}'! {e}")),
        }
    }
}
