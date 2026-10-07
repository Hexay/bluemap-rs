package bluemaprs.fabric;

import bluemaprs.shim.ShimCore;
import bluemaprs.shim.Slf4jLogger;
import de.bluecolored.bluemap.core.logger.Logger;
import net.fabricmc.api.DedicatedServerModInitializer;
import net.fabricmc.fabric.api.command.v2.CommandRegistrationCallback;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerTickEvents;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLevelEvents;
import net.fabricmc.fabric.api.networking.v1.ServerPlayConnectionEvents;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.server.MinecraftServer;
import org.slf4j.LoggerFactory;

import java.nio.file.Path;
import java.util.Optional;

/**
 * The Fabric side of bluemap-rs (docs/14): a dedicated-server entrypoint feeding the loader-neutral {@link ShimCore}.
 * Nothing starts before {@code SERVER_STARTED}; {@code SERVER_STOPPING} waits for the core to save and exit.
 */
public final class BlueMapFabricMod implements DedicatedServerModInitializer {

    private static final org.slf4j.Logger LOG = LoggerFactory.getLogger("BlueMap");

    private FabricPlatform platform;
    private ShimCore<CommandSourceStack> core;
    private FabricPlayers players;
    private volatile boolean running;

    @Override
    public void onInitializeServer() {
        Logger.global.clear();
        Logger.global.put(new Slf4jLogger(LOG));

        Optional<Path> upstream = Coexistence.findUpstream(FabricLoader.getInstance().getGameDir().resolve("mods"));
        if (upstream.isPresent()) {
            LOG.error("Upstream BlueMap is installed as well ({}). Both would render the same maps and corrupt them. "
                    + "Remove one of the two jars; bluemap-rs stays inactive.", upstream.get().getFileName());
            return;
        }

        String version = FabricLoader.getInstance().getModContainer("bluemap").orElseThrow()
                .getMetadata().getVersion().getFriendlyString();
        platform = new FabricPlatform(version);
        core = new ShimCore<>(platform, LOG, FabricSender::new, ready -> {});
        players = new FabricPlayers(core.players());

        CommandRegistrationCallback.EVENT.register((dispatcher, registries, environment) ->
                dispatcher.getRoot().addChild(core.commands().node()));
        ServerLifecycleEvents.SERVER_STARTED.register(this::started);
        ServerLifecycleEvents.SERVER_STOPPING.register(server -> stopping());
        ServerPlayConnectionEvents.JOIN.register((handler, sender, server) -> {
            if (running) players.join(handler.getPlayer());
        });
        ServerPlayConnectionEvents.DISCONNECT.register((handler, server) -> {
            if (running) players.leave(handler.getPlayer());
        });
        ServerTickEvents.END_SERVER_TICK.register(server -> {
            if (running) players.tick(server);
        });
        ServerLevelEvents.LOAD.register((server, level) -> {
            if (running) core.worldAdded(FabricPlatform.worldInfo(level));
        });
        ServerLevelEvents.UNLOAD.register((server, level) -> {
            if (running) core.worldRemoved(FabricPlatform.worldId(level));
        });
    }

    private void started(MinecraftServer server) {
        LOG.info("Saving all worlds once, to make sure all required world-properties are present...");
        server.saveAllChunks(false, true, true);
        platform.started(server);
        running = true;
        core.start();
    }

    private void stopping() {
        if (!running) return;
        running = false;
        LOG.info("Stopping...");
        platform.stopping();
        core.stop();
        LOG.info("Saved and stopped!");
    }

}
