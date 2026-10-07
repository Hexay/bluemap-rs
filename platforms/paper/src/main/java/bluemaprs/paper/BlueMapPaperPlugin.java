package bluemaprs.paper;

import bluemaprs.shim.ShimCore;
import bluemaprs.shim.Slf4jLogger;
import bluemaprs.shim.ipc.Proto.ReadyInfo;
import de.bluecolored.bluemap.core.logger.Logger;
import io.papermc.paper.command.brigadier.CommandSourceStack;
import io.papermc.paper.plugin.lifecycle.event.types.LifecycleEvents;
import org.bstats.bukkit.Metrics;
import org.bukkit.World;
import org.bukkit.plugin.java.JavaPlugin;

import java.nio.file.Path;
import java.util.Optional;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * The Paper side of bluemap-rs (docs/13): Bukkit events and facts feed the loader-neutral {@link ShimCore}; everything
 * else runs in the out-of-process Rust core.
 */
@SuppressWarnings("UnstableApiUsage")
public final class BlueMapPaperPlugin extends JavaPlugin {

    // TODO: register a bStats plugin id
    static final int BSTATS_ID = 0;

    private final AtomicBoolean metricsStarted = new AtomicBoolean();
    private PaperPlatform platform;
    private ShimCore<CommandSourceStack> core;
    private PaperPlayers players;

    public BlueMapPaperPlugin() {
        Logger.global.clear();
        Logger.global.put(new Slf4jLogger(getSLF4JLogger()));
    }

    @Override
    public void onEnable() {
        Optional<Path> upstream = Coexistence.findUpstream(getServer().getPluginsFolder().toPath());
        if (upstream.isPresent()) {
            getSLF4JLogger().error("Upstream BlueMap is installed as well ({}). Both would render the same maps and "
                    + "corrupt them. Remove one of the two jars; bluemap-rs is disabling itself.",
                    upstream.get().getFileName());
            getServer().getPluginManager().disablePlugin(this);
            return;
        }

        if (!PaperPlatform.IS_FOLIA) {
            getSLF4JLogger().info("Saving all worlds once, to make sure all required world-properties are present...");
            for (World world : getServer().getWorlds()) world.save();
        } else {
            getSLF4JLogger().info("Folia detected, enabling folia-support mode.");
        }

        platform = new PaperPlatform(this);
        core = new ShimCore<>(platform, getSLF4JLogger(), PaperSender::new, this::startMetrics);
        players = new PaperPlayers(this, core.players());

        getServer().getPluginManager().registerEvents(players, this);
        getServer().getPluginManager().registerEvents(new WorldEvents(core), this);
        getLifecycleManager().registerEventHandler(LifecycleEvents.COMMANDS,
                event -> event.registrar().register(core.commands().node()));

        players.start();
        core.start();
    }

    @Override
    public void onDisable() {
        if (core == null) return;
        getSLF4JLogger().info("Stopping...");
        platform.stopping();
        players.stop();
        core.stop();
        getSLF4JLogger().info("Saved and stopped!");
    }

    /** Own bStats id only (never upstream's 5912), and only if {@code metrics} is enabled in core.conf. */
    private void startMetrics(ReadyInfo ready) {
        if (BSTATS_ID <= 0 || ready.plugin() == null || !ready.plugin().metrics()) return;
        if (metricsStarted.compareAndSet(false, true)) new Metrics(this, BSTATS_ID);
    }

}
