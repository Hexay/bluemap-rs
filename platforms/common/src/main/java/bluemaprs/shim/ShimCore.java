package bluemaprs.shim;

import bluemaprs.shim.api.Providers;
import bluemaprs.shim.api.ShimBackend;
import bluemaprs.shim.commands.CommandBridge;
import bluemaprs.shim.commands.CommandSpec;
import bluemaprs.shim.core.CoreDispatcher;
import bluemaprs.shim.core.CoreSupervisor;
import bluemaprs.shim.ipc.CoreLink;
import bluemaprs.shim.ipc.Frame;
import bluemaprs.shim.ipc.Msg;
import bluemaprs.shim.ipc.Proto;
import bluemaprs.shim.ipc.Proto.ReadyInfo;
import bluemaprs.shim.load.ServerLoadReporter;
import bluemaprs.shim.markers.MarkerPusher;
import bluemaprs.shim.players.PlayerRegistry;
import bluemaprs.shim.skins.PlayerSkinUpdater;
import com.google.gson.JsonObject;
import org.slf4j.Logger;

import java.io.IOException;
import java.util.function.Consumer;
import java.util.function.Function;

/**
 * The loader-neutral shim (docs/13): IPC link, API backend, markers, players, {@code /bluemap} and the core process.
 * One instance per server run; the platform forwards its events and calls {@link #start}/{@link #stop}.
 *
 * @param <S> the platform's Brigadier command source
 */
public final class ShimCore<S> {

    private final Platform platform;
    private final Logger log;
    private final Consumer<ReadyInfo> onReady;
    private final CoreLink link;
    private final PlayerRegistry players;
    private final MarkerPusher markers;
    private final CommandBridge<S> commands;
    private final PlayerSkinUpdater skins;
    private final ServerLoadReporter load;
    // not final: constructor lambdas capture them before they are assigned
    private ShimBackend backend;
    private CoreSupervisor supervisor;
    private byte[] blockstates;

    /** @param onReady after every successful core load, on the {@code BlueMap-Load} thread */
    public ShimCore(Platform platform, Logger log, Function<S, CommandBridge.Sender> senders,
                    Consumer<ReadyInfo> onReady) {
        this.platform = platform;
        this.onReady = onReady;
        CommandSpec spec;
        try {
            spec = CommandSpec.load(ShimCore.class.getResourceAsStream("/commands.json"));
        } catch (IOException e) {
            throw new IllegalStateException("Broken BlueMap jar", e);
        }

        this.log = log;
        link = new CoreLink();
        players =new PlayerRegistry(link, () -> backend.providers().getPlayerDisplayNameProvider(), log);
        backend = new ShimBackend(link, log, platform::serverWorldId, new Providers(players::accountName));
        markers = new MarkerPusher(link, log);
        commands = new CommandBridge<>(spec, link, backend::mirror, () -> supervisor,
                platform.displayName() + " shim " + platform.shimVersion(), senders);
        CoreDispatcher dispatcher = new CoreDispatcher(log, link, backend, markers, commands, platform::saveWorld);
        supervisor = new CoreSupervisor(log, platform.configFolder(), link, backend, markers, players, dispatcher,
                this::hello, this::ready);
        skins = new PlayerSkinUpdater(backend);
        players.setJoinListener(skins::onPlayerJoin);
        load = new ServerLoadReporter(link, platform::averageTickMillis, log);
    }

    public PlayerRegistry players() {
        return players;
    }

    public CommandBridge<S> commands() {
        return commands;
    }

    /** A world loaded after start (Multiverse, a mod's dimension). */
    public void worldAdded(Proto.WorldInfo world) {
        JsonObject header = Msg.of("WorldAdded");
        header.add("world", Msg.GSON.toJsonTree(world));
        send(header);
    }

    public void worldRemoved(String worldId) {
        JsonObject header = Msg.of("WorldRemoved");
        header.addProperty("id", worldId);
        send(header);
    }

    public void start() {
        players.start();
        markers.start();
        load.start();
        supervisor.start();
    }

    /** Server stop: blocks up to ~35 s while the core saves and exits. */
    public void stop() {
        players.stop();
        load.stop();
        supervisor.stop();
        markers.stop();
        skins.stop();
    }

    private Frame hello() {
        if (blockstates == null) blockstates = platform.defaultBlockstates();
        Proto.Hello hello = new Proto.Hello(
                Msg.PROTOCOL,
                platform.id(),
                platform.minecraftVersion(),
                platform.shimVersion(),
                platform.configFolder().toAbsolutePath().toString(),
                platform.modsFolder(),
                null,
                platform.folia(),
                Runtime.getRuntime().maxMemory() / (1024 * 1024),
                platform.worlds()
        );
        return new Frame(Msg.of("Hello", hello), blockstates);
    }

    private void send(JsonObject header) {
        if (!link.connected()) return; // the next Hello carries the current world list
        try {
            link.send(new Frame(header));
        } catch (IOException e) {
            log.debug("{} not sent: {}", Msg.string(header, "t"), e.getMessage());
        }
    }

    private void ready(ReadyInfo ready) {
        skins.reset();
        onReady.accept(ready);
    }

}
