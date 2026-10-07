package bluemaprs.paper;

import bluemaprs.paper.api.Providers;
import bluemaprs.paper.api.ShimBackend;
import bluemaprs.paper.commands.BlueMapCommand;
import bluemaprs.paper.commands.CommandSpec;
import bluemaprs.paper.core.CoreDispatcher;
import bluemaprs.paper.core.CoreSupervisor;
import bluemaprs.paper.ipc.CoreLink;
import bluemaprs.paper.ipc.Frame;
import bluemaprs.paper.ipc.Msg;
import bluemaprs.paper.ipc.Proto;
import bluemaprs.paper.ipc.Proto.ReadyInfo;
import bluemaprs.paper.markers.MarkerPusher;
import bluemaprs.paper.players.PlayerTracker;
import bluemaprs.paper.skins.PlayerSkinUpdater;
import de.bluecolored.bluemap.core.logger.JavaLogger;
import de.bluecolored.bluemap.core.logger.Logger;
import io.papermc.paper.plugin.lifecycle.event.types.LifecycleEvents;
import org.bstats.bukkit.Metrics;
import org.bukkit.World;
import org.bukkit.plugin.java.JavaPlugin;

import java.io.IOException;
import java.nio.file.Path;
import java.util.Optional;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * The Paper side of bluemap-rs (docs/13): JVM-only facts and callbacks stay here, everything else runs in the
 * out-of-process Rust core.
 */
public final class BlueMapPaperPlugin extends JavaPlugin {

    // TODO: register a bStats plugin id
    static final int BSTATS_ID = 0;

    private final AtomicBoolean metricsStarted = new AtomicBoolean();
    private byte[] blockstates;
    private ShimBackend backend;
    private PlayerTracker players;
    private MarkerPusher markers;
    private WorldEvents worldEvents;
    private CoreSupervisor supervisor;
    private PlayerSkinUpdater skins;

    public BlueMapPaperPlugin() {
        Logger.global.clear();
        Logger.global.put(new JavaLogger(getLogger()));
    }

    @Override
    @SuppressWarnings("UnstableApiUsage")
    public void onEnable() {
        Optional<Path> upstream = Coexistence.findUpstream(getServer().getPluginsFolder().toPath());
        if (upstream.isPresent()) {
            getLogger().severe("Upstream BlueMap is installed as well (" + upstream.get().getFileName() + "). Both "
                    + "would render the same maps and corrupt them. Remove one of the two jars; bluemap-rs is "
                    + "disabling itself.");
            getServer().getPluginManager().disablePlugin(this);
            return;
        }

        if (!ServerInfo.IS_FOLIA) {
            getLogger().info("Saving all worlds once, to make sure all required world-properties are present...");
            for (World world : getServer().getWorlds()) world.save();
        } else {
            getLogger().info("Folia detected, enabling folia-support mode.");
        }

        CommandSpec spec;
        try {
            spec = CommandSpec.load(getResource("commands.json"));
        } catch (IOException e) {
            throw new IllegalStateException("Broken plugin jar", e);
        }

        CoreLink link = new CoreLink();
        players = new PlayerTracker(this, link, () -> backend.providers().getPlayerDisplayNameProvider(), getLogger());
        backend = new ShimBackend(link, getLogger(), ServerInfo::serverWorldId, new Providers(players::accountName));
        markers = new MarkerPusher(link, getLogger());
        worldEvents = new WorldEvents(this, link, getLogger());
        BlueMapCommand command = new BlueMapCommand(spec, link, backend::mirror, () -> supervisor,
                getPluginMeta().getVersion());
        CoreDispatcher dispatcher = new CoreDispatcher(getLogger(), link, backend, markers, command, worldEvents::save);
        supervisor = new CoreSupervisor(getLogger(), getDataFolder().toPath(), link, backend, markers, players,
                dispatcher, this::hello, this::onReady);
        skins = new PlayerSkinUpdater(backend);
        players.setJoinListener(skins::onPlayerJoin);

        getServer().getPluginManager().registerEvents(players, this);
        getServer().getPluginManager().registerEvents(worldEvents, this);
        getLifecycleManager().registerEventHandler(LifecycleEvents.COMMANDS,
                event -> command.register(event.registrar()));

        players.start();
        markers.start();
        supervisor.start();
    }

    @Override
    public void onDisable() {
        if (supervisor == null) return;
        getLogger().info("Stopping...");
        worldEvents.stopping();
        players.stop();
        supervisor.stop();
        markers.stop();
        skins.stop();
        getLogger().info("Saved and stopped!");
    }

    private Frame hello() {
        if (blockstates == null) blockstates = ServerInfo.defaultBlockstates(getLogger());
        Proto.Hello hello = new Proto.Hello(
                Msg.PROTOCOL,
                "paper",
                ServerInfo.minecraftVersion(),
                getPluginMeta().getVersion(),
                getDataFolder().getAbsolutePath(),
                "mods",
                null,
                ServerInfo.IS_FOLIA,
                Runtime.getRuntime().maxMemory() / (1024 * 1024),
                ServerInfo.worlds()
        );
        return new Frame(Msg.of("Hello", hello), blockstates);
    }

    private void onReady(ReadyInfo ready) {
        skins.reset();
        startMetrics(ready);
    }

    /** Own bStats id only (never upstream's 5912), and only if {@code metrics} is enabled in core.conf. */
    private void startMetrics(ReadyInfo ready) {
        if (BSTATS_ID <= 0 || ready.plugin() == null || !ready.plugin().metrics()) return;
        if (metricsStarted.compareAndSet(false, true)) new Metrics(this, BSTATS_ID);
    }

}
