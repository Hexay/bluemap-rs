package bluemaprs.paper.api;

import bluemaprs.paper.ipc.CoreLink;
import bluemaprs.paper.ipc.Msg;
import bluemaprs.paper.ipc.Proto.ReadyInfo;
import bluemaprs.paper.ipc.Proto.StateInfo;
import bluemaprs.paper.ipc.Reply;
import com.google.gson.JsonObject;

import java.io.IOException;
import java.io.InterruptedIOException;
import java.nio.file.Path;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.function.Function;
import java.util.logging.Logger;

/**
 * Everything the {@code de.bluecolored.bluemap.common.api} impls need: the mirror, RPCs (5 s timeout) and the
 * JVM-side state that is replayed to a restarted core (scripts/styles, frozen/visibility changes made while down).
 */
public final class ShimBackend {

    public static final long RPC_TIMEOUT_MS = 5000;

    private final CoreLink link;
    private final Logger log;
    private final Function<Object, Optional<String>> serverWorldResolver;
    private final Providers providers;

    private volatile Mirror mirror = Mirror.EMPTY;
    private final Set<String> scripts = Collections.synchronizedSet(new LinkedHashSet<>());
    private final Set<String> styles = Collections.synchronizedSet(new LinkedHashSet<>());
    private final Map<String, Boolean> pendingFrozen = new ConcurrentHashMap<>();
    private final Map<UUID, Boolean> pendingVisibility = new ConcurrentHashMap<>();

    public ShimBackend(CoreLink link, Logger log, Function<Object, Optional<String>> serverWorldResolver,
                       Providers providers) {
        this.link = link;
        this.log = log;
        this.serverWorldResolver = serverWorldResolver;
        this.providers = providers;
    }

    public Mirror mirror() {
        return mirror;
    }

    public synchronized void setReady(ReadyInfo ready) {
        mirror = ready == null ? Mirror.EMPTY : Mirror.of(ready);
    }

    public synchronized void updateState(StateInfo state) {
        mirror = mirror.withState(state);
    }

    public Providers providers() {
        return providers;
    }

    /** Bukkit {@code World}, UUID, world name or key → {@code WorldInfo.id}. */
    public Optional<String> serverWorldId(Object world) {
        return serverWorldResolver.apply(world);
    }

    public Path webroot() {
        ReadyInfo ready = mirror.ready();
        return ready != null && ready.webroot() != null ? Path.of(ready.webroot()) : Path.of("bluemap", "web");
    }

    /** Sends {@code header} (an id is added) and waits for an ok {@code Reply}. */
    public Reply call(JsonObject header, byte[] body) throws IOException {
        String type = Msg.string(header, "t");
        try {
            Reply reply = link.request(header, body).get(RPC_TIMEOUT_MS, TimeUnit.MILLISECONDS);
            if (!reply.ok()) throw new IOException(reply.err() != null ? reply.err() : type + " failed");
            return reply;
        } catch (ExecutionException e) {
            throw e.getCause() instanceof IOException io ? io : new IOException(e.getCause());
        } catch (TimeoutException e) {
            throw new IOException("BlueMap core did not answer " + type + " within " + RPC_TIMEOUT_MS + " ms");
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new InterruptedIOException("interrupted waiting for " + type);
        }
    }

    public Reply call(JsonObject header) throws IOException {
        return call(header, new byte[0]);
    }

    public boolean isFrozen(String map) {
        Boolean pending = pendingFrozen.get(map);
        return pending != null ? pending : mirror.state().frozenMaps().contains(map);
    }

    public void setFrozen(String map, boolean frozen) {
        JsonObject header = Msg.of("SetFrozen");
        header.addProperty("map", map);
        header.addProperty("frozen", frozen);
        if (!link.connected()) {
            pendingFrozen.put(map, frozen);
            return;
        }
        try {
            call(header);
            synchronized (this) { mirror = mirror.withFrozen(map, frozen); }
        } catch (IOException e) {
            log.warning("Failed to " + (frozen ? "freeze" : "unfreeze") + " map '" + map + "': " + e.getMessage());
        }
    }

    public boolean getPlayerVisibility(UUID player) {
        Boolean pending = pendingVisibility.get(player);
        return pending != null ? pending : !mirror.state().hiddenPlayers().contains(player.toString());
    }

    public void setPlayerVisibility(UUID player, boolean visible) {
        if (!link.connected()) {
            pendingVisibility.put(player, visible);
            return;
        }
        try {
            call(visibilityHeader(player, visible));
            synchronized (this) { mirror = mirror.withHidden(player.toString(), !visible); }
        } catch (IOException e) {
            log.warning("Failed to set the visibility of player " + player + ": " + e.getMessage());
        }
    }

    public void registerScript(String url) {
        if (scripts.add(url)) sendAsync(webAsset("RegisterScript", url));
    }

    public void registerStyle(String url) {
        if (styles.add(url)) sendAsync(webAsset("RegisterStyle", url));
    }

    /** API generation ended: its plugins re-register their scripts/styles in {@code onEnable}. */
    public void clearWebAssets() {
        scripts.clear();
        styles.clear();
    }

    /** A restarted core lost the JVM-side state: resend scripts/styles and the changes made while it was down. */
    public void replayAll() {
        List.copyOf(scripts).forEach(url -> sendAsync(webAsset("RegisterScript", url)));
        List.copyOf(styles).forEach(url -> sendAsync(webAsset("RegisterStyle", url)));
        replayPending();
    }

    public void replayPending() {
        Map<String, Boolean> frozen = Map.copyOf(pendingFrozen);
        pendingFrozen.keySet().removeAll(frozen.keySet());
        frozen.forEach(this::setFrozen);
        Map<UUID, Boolean> visibility = Map.copyOf(pendingVisibility);
        pendingVisibility.keySet().removeAll(visibility.keySet());
        visibility.forEach(this::setPlayerVisibility);
    }

    private void sendAsync(JsonObject header) {
        if (!link.connected()) return;
        link.request(header, new byte[0]).whenComplete((reply, e) -> {
            String err = e != null ? e.getMessage() : reply.ok() ? null : reply.err();
            if (err != null) log.warning(Msg.string(header, "t") + " failed: " + err);
        });
    }

    private static JsonObject webAsset(String type, String url) {
        JsonObject header = Msg.of(type);
        header.addProperty("url", url);
        return header;
    }

    private static JsonObject visibilityHeader(UUID player, boolean visible) {
        JsonObject header = Msg.of("SetPlayerVisibility");
        header.addProperty("uuid", player.toString());
        header.addProperty("visible", visible);
        return header;
    }

}
