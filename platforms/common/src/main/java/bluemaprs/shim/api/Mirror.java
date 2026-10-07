package bluemaprs.shim.api;

import bluemaprs.shim.ipc.Proto.CoreWorld;
import bluemaprs.shim.ipc.Proto.MapInfo;
import bluemaprs.shim.ipc.Proto.ReadyInfo;
import bluemaprs.shim.ipc.Proto.StateInfo;

import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Objects;
import java.util.Optional;
import java.util.Set;

/** Immutable snapshot of the core's last {@code Ready} plus later {@code StateChanged}s; API reads never block. */
public record Mirror(ReadyInfo ready, StateInfo state) {

    public static final Mirror EMPTY = new Mirror(null, StateInfo.EMPTY);

    public static Mirror of(ReadyInfo ready) {
        return new Mirror(ready, ready.state());
    }

    public List<MapInfo> maps() {
        return ready == null ? List.of() : ready.maps();
    }

    public List<CoreWorld> worlds() {
        return ready == null ? List.of() : ready.worlds();
    }

    public List<String> storages() {
        return ready == null ? List.of() : ready.storages();
    }

    public Set<String> mapIds() {
        Set<String> ids = new LinkedHashSet<>();
        for (MapInfo map : maps()) ids.add(map.id());
        return ids;
    }

    public Optional<MapInfo> map(String id) {
        return maps().stream().filter(m -> m.id().equals(id)).findFirst();
    }

    public Optional<CoreWorld> world(String id) {
        return worlds().stream().filter(w -> w.id().equals(id)).findFirst();
    }

    public Optional<CoreWorld> worldOfServerWorld(String serverWorldId) {
        return worlds().stream().filter(w -> Objects.equals(w.serverWorld(), serverWorldId)).findFirst();
    }

    public Mirror withState(StateInfo state) {
        return new Mirror(ready, state);
    }

    public Mirror withFrozen(String map, boolean frozen) {
        return withState(new StateInfo(toggle(state.frozenMaps(), map, frozen), state.hiddenPlayers(),
                state.renderThreadsRunning()));
    }

    public Mirror withHidden(String uuid, boolean hidden) {
        return withState(new StateInfo(state.frozenMaps(), toggle(state.hiddenPlayers(), uuid, hidden),
                state.renderThreadsRunning()));
    }

    public Mirror withRenderThreadsRunning(boolean running) {
        return withState(new StateInfo(state.frozenMaps(), state.hiddenPlayers(), running));
    }

    private static List<String> toggle(List<String> list, String value, boolean present) {
        List<String> copy = new ArrayList<>(list);
        copy.remove(value);
        if (present) copy.add(value);
        return List.copyOf(copy);
    }

}
