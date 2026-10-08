package bluemaprs.shim.markers;

import bluemaprs.shim.api.MarkerStore;
import bluemaprs.shim.ipc.CoreLink;
import bluemaprs.shim.ipc.Frame;
import bluemaprs.shim.ipc.Msg;
import com.google.gson.JsonObject;
import de.bluecolored.bluemap.api.gson.MarkerGson;
import org.slf4j.Logger;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

/**
 * Demand-driven live markers (docs/13 §3, §7.7): every 10 s each demanded map is serialized exactly like upstream's
 * {@code LiveMarkersDataSupplier} and pushed when the JSON changed.
 */
public final class MarkerPusher {

    public static final int CHUNK = 1 << 20;
    private static final long INTERVAL_S = 10;

    private final CoreLink link;
    private final Logger log;
    private final ScheduledExecutorService timer = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread thread = new Thread(r, "BlueMap-Markers");
        thread.setDaemon(true);
        return thread;
    });
    private final Set<String> demand = ConcurrentHashMap.newKeySet();
    private final Map<String, String> lastPushed = new ConcurrentHashMap<>();
    private volatile MarkerStore store;

    public MarkerPusher(CoreLink link, Logger log) {
        this.link = link;
        this.log = log;
    }

    public void start() {
        timer.scheduleWithFixedDelay(this::tick, INTERVAL_S, INTERVAL_S, TimeUnit.SECONDS);
    }

    public void stop() {
        timer.shutdownNow();
    }

    /** A new API generation; pushes nothing by itself (see {@link #replayAll}). */
    public void setStore(MarkerStore store) {
        this.store = store;
        lastPushed.clear();
    }

    public void setDemand(Collection<String> maps) {
        List<String> added = maps.stream().filter(map -> !demand.contains(map)).toList();
        demand.retainAll(maps);
        demand.addAll(maps);
        if (!added.isEmpty()) timer.execute(() -> added.forEach(map -> push(map, true)));
    }

    /** Pushes every map once, e.g. to a restarted core or after plugins filled a fresh store. */
    public void replayAll() {
        timer.execute(this::pushAll);
    }

    /** Synchronous final push before {@code Shutdown}, regardless of demand. */
    public void flushAll() {
        pushAll();
    }

    private void tick() {
        demand.forEach(map -> push(map, false));
    }

    private synchronized void pushAll() {
        MarkerStore s = store;
        if (s != null) s.mapIds().forEach(map -> push(map, true));
    }

    private synchronized void push(String map, boolean force) {
        MarkerStore s = store;
        if (s == null || !link.connected()) return;
        try {
            String json = MarkerGson.INSTANCE.toJson(s.sets(map));
            if (!force && json.equals(lastPushed.get(map))) return;
            link.sendLatest("markers:" + map, frames(map, json.getBytes(StandardCharsets.UTF_8)));
            lastPushed.put(map, json);
        } catch (RuntimeException e) {
            log.warn("Failed to serialize the markers of map '" + map + "'", e);
        }
    }

    static List<Frame> frames(String map, byte[] json) {
        List<Frame> frames = new ArrayList<>();
        int offset = 0;
        do {
            int end = Math.min(json.length, offset + CHUNK);
            JsonObject header = Msg.of("Markers");
            header.addProperty("map", map);
            header.addProperty("more", end < json.length);
            frames.add(new Frame(header, Arrays.copyOfRange(json, offset, end)));
            offset = end;
        } while (offset < json.length);
        return frames;
    }

}
