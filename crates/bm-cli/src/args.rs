//! BlueMapCLI's options (`BlueMapCLI.createOptions`): same short and long names, same arguments.

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug, Default)]
#[command(name = "bluemap", disable_help_flag = true, disable_version_flag = true)]
pub struct Args {
    #[arg(short = 'h', long = "help")]
    pub help: bool,
    #[arg(short = 'c', long = "config", value_name = "config-folder")]
    pub config: Option<PathBuf>,
    #[arg(short = 'n', long = "mods", value_name = "mods-folder")]
    pub mods: Option<PathBuf>,
    #[arg(short = 'v', long = "mc-version", value_name = "mc-version")]
    pub mc_version: Option<String>,
    #[arg(short = 'l', long = "log-file", value_name = "file-name")]
    pub log_file: Option<PathBuf>,
    #[arg(short = 'a', long = "append")]
    pub append: bool,
    #[arg(short = 'w', long = "webserver")]
    pub webserver: bool,
    #[arg(short = 'b', long = "verbose")]
    pub verbose: bool,
    #[arg(short = 'g', long = "generate-webapp")]
    pub generate_webapp: bool,
    #[arg(short = 's', long = "generate-websettings")]
    pub generate_websettings: bool,
    #[arg(short = 'r', long = "render")]
    pub render: bool,
    #[arg(short = 'e', long = "fix-edges")]
    pub fix_edges: bool,
    #[arg(short = 'f', long = "force-render")]
    pub force_render: bool,
    #[arg(short = 'm', long = "maps", value_name = "arg")]
    pub maps: Option<String>,
    #[arg(long = "markers")]
    pub markers: bool,
    #[arg(short = 'u', long = "watch")]
    pub watch: bool,
    #[arg(short = 'V', long = "version")]
    pub version: bool,
    /// bluemap-rs only: convert this storage in place to the `--to` format.
    #[arg(long = "convert-storage", value_name = "storage-id", requires = "to")]
    pub convert_storage: Option<String>,
    #[arg(long = "to", value_name = "format", requires = "convert_storage")]
    pub to: Option<String>,
}

impl Args {
    /// `-r`, `-f`, `-u` and `-e` all start a render.
    pub fn renders(&self) -> bool {
        self.render || self.force_render || self.watch || self.fix_edges
    }
}

pub const HELP: &str = "\
usage: bluemap [options]

Options:
 -a,--append                     Causes log save file to be appended
                                 rather than replaced.
 -b,--verbose                    Causes the web-server to log requests to
                                 the console
 -c,--config <config-folder>     Sets path of the folder containing the
                                 configuration-files to use
                                 (configurations will be generated here if
                                 they don't exist)
    --convert-storage <storage-id>
                                 Converts that storage in place to the
                                 format given by --to (compat or
                                 optimized) and updates its config.
                                 Stop BlueMap first.
 -e,--fix-edges                  Forces rendering the map-edges, instead
                                 of only rendering chunks that have been
                                 modified since the last render
 -f,--force-render               Forces rendering everything, instead of
                                 only rendering chunks that have been
                                 modified since the last render
 -g,--generate-webapp            Generates the files for the web-app to
                                 the folder configured in the
                                 'webapp.conf' file
 -h,--help                       Displays this message
 -l,--log-file <file-name>       Sets a file to save the log to. If not
                                 specified, no log will be saved.
 -m,--maps <arg>                 A comma-separated list of map-id's that
                                 should be rendered. Example:
                                 'world,nether'
    --markers                    Updates the map-markers based on the map
                                 configs
 -n,--mods <mods-folder>         Sets path of the folder containing the
                                 mods that contain extra resources for
                                 rendering.
 -r,--render                     Renders the maps configured in the
                                 'render.conf' file
    --to <format>                The target format of --convert-storage
 -s,--generate-websettings       Updates the settings.json for the
                                 web-app
 -u,--watch                      Watches for file-changes after rendering
                                 and updates the map
 -V,--version                    Print the current BlueMap version
 -v,--mc-version <mc-version>    Sets the minecraft-version, used e.g. to
                                 load resource-packs correctly. Defaults
                                 to the latest compatible version.
 -w,--webserver                  Starts the web-server, configured in the
                                 'webserver.conf' file

Examples:

bluemap -c './config/'
Generates the default/example configurations in a folder named 'config' if they are not already present

bluemap -r
Render the configured maps

bluemap -w
Start only the webserver without doing anything else

bluemap -ru
Render the configured maps and then keeps watching the world-files and updates the map once something changed.
";
