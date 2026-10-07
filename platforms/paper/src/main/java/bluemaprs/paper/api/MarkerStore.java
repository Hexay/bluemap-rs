package bluemaprs.paper.api;

import de.bluecolored.bluemap.api.markers.MarkerSet;

import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/**
 * The marker sets of every map for one API generation: survives {@code BlueMapMapImpl} instances and core
 * restarts, replaced on reload (plugins re-add their markers in {@code onEnable}).
 */
public final class MarkerStore {

    private final Map<String, Map<String, MarkerSet>> maps = new ConcurrentHashMap<>();

    /** The live, modifiable map handed out by {@code BlueMapMap.getMarkerSets()}. */
    public Map<String, MarkerSet> sets(String mapId) {
        return maps.computeIfAbsent(mapId, id -> new ConcurrentHashMap<>());
    }

    public Set<String> mapIds() {
        return maps.keySet();
    }

}
