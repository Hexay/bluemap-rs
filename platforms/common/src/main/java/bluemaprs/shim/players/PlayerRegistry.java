package bluemaprs.shim.players;

import bluemaprs.shim.ipc.CoreLink;
import bluemaprs.shim.ipc.Frame;
import bluemaprs.shim.ipc.Msg;
import bluemaprs.shim.ipc.Proto.PlayerInfo;
import com.google.gson.JsonObject;
import de.bluecolored.bluemap.api.plugin.PlayerDisplayNameProvider;
import org.slf4j.Logger;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;
import java.util.function.Supplier;

/**
 * Online players as upstream's plugins track them: the platform refreshes each player's snapshot about once a second
 * on the thread that owns it; this sends one {@code Players} batch per second. The core applies every hide rule.
 */
public final class PlayerRegistry {

    private final CoreLink link;
    private final Supplier<PlayerDisplayNameProvider> displayNames;
    private final Logger log;
    /** Iteration order of this map is upstream's live-players JSON order. */
    private final Map<UUID, PlayerSnapshot> online = new ConcurrentHashMap<>();
    private volatile Consumer<UUID> joinListener = uuid -> {};
    private final ScheduledExecutorService sender = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread thread = new Thread(r, "BlueMap-Players");
        thread.setDaemon(true);
        return thread;
    });

    public PlayerRegistry(CoreLink link, Supplier<PlayerDisplayNameProvider> displayNames, Logger log) {
        this.link = link;
        this.displayNames = displayNames;
        this.log = log;
    }

    public void start() {
        sender.scheduleAtFixedRate(this::sendBatch, 1, 1, TimeUnit.SECONDS);
    }

    public void stop() {
        sender.shutdownNow();
        online.clear();
    }

    public void setJoinListener(Consumer<UUID> listener) {
        this.joinListener = listener;
    }

    /** A player already online when tracking starts (or after a respawn): no {@code PlayerJoin}. */
    public void track(PlayerSnapshot snapshot) {
        online.put(snapshot.uuid(), snapshot);
    }

    public void join(PlayerSnapshot snapshot) {
        track(snapshot);
        sendUuid("PlayerJoin", snapshot.uuid());
        joinListener.accept(snapshot.uuid());
    }

    /** A periodic refresh; ignored once the player left. */
    public void update(PlayerSnapshot snapshot) {
        online.replace(snapshot.uuid(), snapshot);
    }

    public void leave(UUID uuid) {
        online.remove(uuid);
        sendUuid("PlayerLeave", uuid);
    }

    /** Leaves every tracked player missing from {@code online}: some disconnects (fake players) fire no event. */
    public void retain(Set<UUID> online) {
        for (UUID uuid : List.copyOf(this.online.keySet())) {
            if (!online.contains(uuid)) leave(uuid);
        }
    }

    /** Upstream's default {@code PlayerDisplayNameProvider}. */
    public String accountName(UUID uuid) {
        PlayerSnapshot snapshot = online.get(uuid);
        return snapshot != null ? snapshot.accountName() : uuid.toString();
    }

    /** After a core restart: send the current batch without waiting for the next tick. */
    public void resend() {
        sender.execute(this::sendBatch);
    }

    private void sendBatch() {
        if (!link.connected()) return;
        try {
            PlayerDisplayNameProvider names = displayNames.get();
            List<PlayerInfo> players = new ArrayList<>(online.size());
            for (PlayerSnapshot snapshot : online.values()) players.add(snapshot.toInfo(displayName(names, snapshot)));
            JsonObject header = Msg.of("Players");
            header.add("players", Msg.GSON.toJsonTree(players));
            link.sendLatest("players", List.of(new Frame(header)));
        } catch (RuntimeException e) {
            log.warn("Failed to send the online players to the BlueMap core", e);
        }
    }

    private String displayName(PlayerDisplayNameProvider names, PlayerSnapshot snapshot) {
        try {
            String name = names.get(snapshot.uuid());
            return name != null ? name : snapshot.accountName();
        } catch (RuntimeException e) {
            log.debug("PlayerDisplayNameProvider failed", e);
            return snapshot.accountName();
        }
    }

    private void sendUuid(String type, UUID uuid) {
        if (!link.connected()) return;
        JsonObject header = Msg.of(type);
        header.addProperty("uuid", uuid.toString());
        try {
            link.send(new Frame(header));
        } catch (IOException e) {
            log.debug("{} not sent: {}", type, e.getMessage());
        }
    }

}
