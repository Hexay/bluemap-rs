package bluemaprs.paper.core;

import bluemaprs.paper.api.ShimBackend;
import bluemaprs.paper.commands.CoreControl;
import bluemaprs.paper.ipc.CoreConnection;
import bluemaprs.paper.ipc.CoreLink;
import bluemaprs.paper.ipc.Frame;
import bluemaprs.paper.ipc.Msg;
import bluemaprs.paper.ipc.Proto.NotReady;
import bluemaprs.paper.ipc.Proto.ReadyInfo;
import bluemaprs.paper.markers.MarkerPusher;
import bluemaprs.paper.players.PlayerTracker;
import com.google.gson.JsonObject;

import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.Deque;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.Executors;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.function.Consumer;
import java.util.function.Supplier;
import java.util.logging.Logger;

/**
 * Owns the core process: spawn + handshake, crash respawn with backoff (1, 2, 4 … 60 s; gives up after 5 crashes in
 * 10 min), and shutdown. All lifecycle work runs on one {@code BlueMap-Load} thread, so API consumers run there.
 */
public final class CoreSupervisor implements CoreControl {

    private static final long HANDSHAKE_TIMEOUT_S = 30;
    private static final long SHUTDOWN_TIMEOUT_S = 30;
    private static final long CRASH_WINDOW_MS = TimeUnit.MINUTES.toMillis(10);
    private static final int MAX_CRASHES = 5;
    private static final long MAX_BACKOFF_S = 60;

    private final Logger log;
    private final Path dataFolder;
    private final CoreLink link;
    private final CoreBinary binary;
    private final CoreDispatcher dispatcher;
    private final MarkerPusher markers;
    private final ApiLifecycle api;
    private final Supplier<Frame> hello;
    private final ScheduledExecutorService lifecycle = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread thread = new Thread(r, "BlueMap-Load");
        thread.setDaemon(true);
        return thread;
    });

    // lifecycle thread only
    private final Deque<Long> crashes = new ArrayDeque<>();
    private ScheduledFuture<?> pendingRespawn;
    private boolean reconnecting, forceCycle;

    private volatile boolean stopping, gaveUp;
    private CoreConnection connection; // guarded by this
    private volatile CompletableFuture<JsonObject> handshake = new CompletableFuture<>();

    public CoreSupervisor(Logger log, Path dataFolder, CoreLink link, ShimBackend backend, MarkerPusher markers,
                          PlayerTracker players, CoreDispatcher dispatcher, Supplier<Frame> hello,
                          Consumer<ReadyInfo> onReady) {
        this.log = log;
        this.dataFolder = dataFolder;
        this.link = link;
        this.binary = new CoreBinary(dataFolder, log);
        this.dispatcher = dispatcher;
        this.markers = markers;
        this.api = new ApiLifecycle(log, backend, markers, players, onReady);
        this.hello = hello;
    }

    public void start() {
        onLifecycle(this::launch);
    }

    @Override
    public boolean willRestart() {
        return !gaveUp && !stopping;
    }

    @Override
    public void restartNow() {
        onLifecycle(() -> {
            synchronized (this) {
                if (connection != null || stopping) return;
            }
            if (pendingRespawn != null) pendingRespawn.cancel(false);
            crashes.clear();
            gaveUp = false;
            reconnecting = true;
            forceCycle = true;
            launch();
        });
    }

    /** Server stop (main thread): unregister the API, flush markers, {@code Shutdown}, wait ≤ 30 s, then kill. */
    public void stop() {
        CoreConnection c;
        synchronized (this) {
            stopping = true;
            c = connection;
        }
        lifecycle.shutdownNow();
        try {
            lifecycle.awaitTermination(5, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
        api.unregister();
        if (c == null) return;

        if (link.connected()) markers.flushAll();
        try {
            c.send(new Frame(Msg.of("Shutdown")));
        } catch (IOException ignored) {
            // already gone
        }
        c.closeInput();
        try {
            if (!c.process().waitFor(SHUTDOWN_TIMEOUT_S, TimeUnit.SECONDS)) {
                log.warning("The BlueMap core did not stop within " + SHUTDOWN_TIMEOUT_S + " s; killing it");
                CoreLauncher.kill(c.process());
            }
        } catch (InterruptedException e) {
            CoreLauncher.kill(c.process());
            Thread.currentThread().interrupt();
        }
        link.detach(c);
    }

    private void launch() {
        pendingRespawn = null;
        if (stopping) return;
        CoreConnection c;
        Path executable;
        try {
            CoreLauncher.killStale(dataFolder, log);
            executable = binary.resolve();
            synchronized (this) {
                if (stopping) return;
                handshake = new CompletableFuture<>();
                c = new CoreConnection(spawn(executable), this::onFrame, log);
                connection = c;
            }
        } catch (CoreUnavailableException e) {
            log.severe(e.getMessage());
            gaveUp = true;
            return;
        }
        c.exit().thenAccept(exit -> onLifecycle(() -> exited(c, exit)));
        c.start();

        try {
            c.send(hello.get());
            JsonObject reply = handshake.get(HANDSHAKE_TIMEOUT_S, TimeUnit.SECONDS);
            if ("Incompatible".equals(Msg.string(reply, "t"))) {
                log.severe("The bundled BlueMap core " + Msg.string(reply, "coreVersion") + " speaks IPC protocol "
                        + reply.get("protocol") + ", this plugin speaks " + Msg.PROTOCOL + ". The jar is broken; "
                        + "re-download it (or fix " + CoreBinary.OVERRIDE + ").");
                gaveUp = true;
                CoreLauncher.kill(c.process());
                return;
            }
            log.info("BlueMap core " + Msg.string(reply, "coreVersion") + " started (pid " + reply.get("pid") + ")");
            link.attach(c);
            binary.deleteOtherVersions(executable);
        } catch (IOException | ExecutionException | TimeoutException e) {
            log.severe("The BlueMap core did not complete the IPC handshake: " + e);
            CoreLauncher.kill(c.process());
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    private static Process spawn(Path executable) throws CoreUnavailableException {
        try {
            return CoreLauncher.spawn(executable);
        } catch (IOException e) {
            throw new CoreUnavailableException("Failed to start the BlueMap core " + executable + " (" + e.getMessage()
                    + "). The server folder may be mounted noexec.", e);
        }
    }

    /** IPC reader thread. */
    private void onFrame(Frame frame) {
        switch (frame.type()) {
            case "Welcome", "Incompatible" -> handshake.complete(frame.header());
            case "Ready" -> {
                ReadyInfo ready = Msg.parse(frame.header(), ReadyInfo.class);
                onLifecycle(() -> {
                    boolean reconnect = reconnecting, cycle = forceCycle;
                    reconnecting = forceCycle = false;
                    api.ready(ready, reconnect, cycle);
                });
            }
            case "NotReady" -> {
                NotReady notReady = Msg.parse(frame.header(), NotReady.class);
                onLifecycle(() -> {
                    reconnecting = forceCycle = false;
                    api.notReady(notReady);
                });
            }
            case "Unloading" -> onLifecycle(api::unregister);
            default -> dispatcher.handle(frame);
        }
    }

    private void exited(CoreConnection c, CoreConnection.Exit exit) {
        synchronized (this) {
            if (connection != c) return;
            connection = null;
        }
        link.detach(c);
        dispatcher.coreGone();
        if (stopping) return;
        if (exit.bye()) {
            log.warning("The BlueMap core stopped (exit code " + exit.code() + "); run /bluemap reload to start it again.");
            return;
        }

        log.severe("The BlueMap core exited unexpectedly (exit code " + exit.code() + ")");
        long now = System.currentTimeMillis();
        crashes.addLast(now);
        while (now - crashes.peekFirst() > CRASH_WINDOW_MS) crashes.pollFirst();
        if (gaveUp) return;
        if (crashes.size() >= MAX_CRASHES) {
            gaveUp = true;
            log.severe("The BlueMap core crashed " + crashes.size() + " times within 10 minutes; not restarting it. "
                    + "Fix the cause (see the log above), then run /bluemap reload to start it again.");
            return;
        }
        long delay = Math.min(MAX_BACKOFF_S, 1L << (crashes.size() - 1));
        log.warning("Restarting the BlueMap core in " + delay + " s");
        reconnecting = true;
        pendingRespawn = lifecycle.schedule(this::launch, delay, TimeUnit.SECONDS);
    }

    private void onLifecycle(Runnable task) {
        try {
            lifecycle.execute(task);
        } catch (RejectedExecutionException ignored) {
            // stopping
        }
    }

}
