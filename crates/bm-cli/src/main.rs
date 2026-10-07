//! `bluemap`: drop-in for `java -jar bluemap-cli.jar` (`BlueMapCLI.main`): same options, config folder handling and
//! exit codes (1 configuration/IO error, 2 missing resources).

mod args;
mod convert;
mod log;
mod render;
mod web;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use args::{Args, HELP};
use bm_config::{BlueMapConfig, ConfigOptions};
use bm_engine::{ResourceOptions, Service, TileUpdateStrategy};
use clap::Parser;

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            log::error(&format!("Failed to parse provided arguments! {}", e.to_string().trim()));
            print!("{HELP}");
            return ExitCode::from(1);
        }
    };
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
        log::add_file(file, args.append).with_context(|| format!("log file {}", file.display()))?;
    }
    if args.help {
        print!("{HELP}");
        return Ok(ExitCode::SUCCESS);
    }
    if args.version {
        println!("{}\nbluemap-rs {}", bm_engine::BLUEMAP_VERSION, env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    let config_folder = config_folder(args);
    std::fs::create_dir_all(&config_folder).with_context(|| format!("create {}", config_folder.display()))?;
    if let Some(mods) = &args.mods
        && !mods.is_dir()
    {
        bail!("Mods folder does not exist: {}", mods.display());
    }
    let unsupported = args.unsupported();
    if !unsupported.is_empty() {
        bail!("not supported by this bluemap-rs build yet: {}", unsupported.join(", "));
    }
    let packs = config_folder.join("packs");
    std::fs::create_dir_all(&packs).with_context(|| format!("create {}", packs.display()))?;

    let config = BlueMapConfig::load(&ConfigOptions::cli(&config_folder))?;
    if let Some(file) = &config.core.log.file {
        // Java formats this path with String.format(file, now); only plain paths are supported so far
        if !file.contains('%') {
            log::add_file(Path::new(file), config.core.log.append).with_context(|| format!("log file {file}"))?;
        }
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

    let webserver = args.webserver.then(|| web::start(&service, args.verbose)).transpose()?;
    let mut ok = true;
    if args.renders() {
        let strategy = if args.force_render {
            TileUpdateStrategy::ForceAll
        } else if args.fix_edges {
            TileUpdateStrategy::ForceEdge
        } else {
            TileUpdateStrategy::ForceNone
        };
        ok = render::render_maps(&service, strategy, args.maps.as_deref(), args.generate_webapp)?;
    } else if args.generate_webapp || args.generate_websettings {
        if args.generate_webapp {
            bm_web::install_webapp(&service.config.webapp.webroot, true)?;
        }
        service.write_webapp_settings()?;
    }
    if let Some(server) = webserver {
        server.wait()?;
    }
    if args.renders() || args.webserver || args.generate_webapp || args.generate_websettings {
        return Ok(if ok { ExitCode::SUCCESS } else { ExitCode::from(1) });
    }
    log::info(&format!(
        "Generated default config files for you, here: {}\n",
        absolute(&config_folder).display()
    ));
    print!("{HELP}");
    Ok(ExitCode::from(1))
}
