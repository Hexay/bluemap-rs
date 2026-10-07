package de.bluecolored.bluemap.common.api;

import bluemaprs.shim.api.ShimBackend;
import bluemaprs.shim.ipc.Msg;
import com.flowpowered.math.vector.Vector2i;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import de.bluecolored.bluemap.api.BlueMapMap;
import de.bluecolored.bluemap.api.RenderManager;
import de.bluecolored.bluemap.core.logger.Logger;

import java.io.IOException;
import java.util.Collection;
import java.util.Objects;
import java.util.Optional;

/** Every call is an RPC; failures log and return the neutral value, as the signatures allow no exception. */
public class RenderManagerImpl implements RenderManager {

    private final ShimBackend backend;

    RenderManagerImpl(ShimBackend backend) {
        this.backend = backend;
    }

    @Override
    public boolean scheduleMapUpdateTask(BlueMapMap map, boolean force) {
        return schedule(map, null, force);
    }

    @Override
    public boolean scheduleMapUpdateTask(BlueMapMap map, Collection<Vector2i> regions, boolean force) {
        JsonArray list = new JsonArray();
        for (Vector2i region : regions) {
            JsonArray xz = new JsonArray();
            xz.add(region.getX());
            xz.add(region.getY());
            list.add(xz);
        }
        return schedule(map, list, force);
    }

    @Override
    public boolean scheduleMapPurgeTask(BlueMapMap map) {
        JsonObject header = Msg.of("Purge");
        header.addProperty("map", Objects.requireNonNull(map, "map").getId());
        return callBool(header);
    }

    @Override
    public int renderQueueSize() {
        return status("queueSize").map(JsonElement::getAsInt).orElse(0);
    }

    @Override
    public int renderThreadCount() {
        return status("threads").map(JsonElement::getAsInt).orElse(0);
    }

    @Override
    public boolean isRunning() {
        return status("running").map(JsonElement::getAsBoolean)
                .orElseGet(() -> backend.mirror().state().renderThreadsRunning());
    }

    @Override
    public void start() {
        callBool(Msg.of("RenderStart"));
    }

    @Override
    public void start(int threadCount) {
        JsonObject header = Msg.of("RenderStart");
        header.addProperty("threads", threadCount);
        callBool(header);
    }

    @Override
    public void stop() {
        callBool(Msg.of("RenderStop"));
    }

    private boolean schedule(BlueMapMap map, JsonArray regions, boolean force) {
        JsonObject header = Msg.of("Schedule");
        header.addProperty("map", Objects.requireNonNull(map, "map").getId());
        if (regions != null) header.add("regions", regions);
        header.addProperty("force", force);
        return callBool(header);
    }

    private Optional<JsonElement> status(String field) {
        try {
            JsonElement value = backend.call(Msg.of("RenderStatus")).value();
            return value.isJsonObject() ? Optional.ofNullable(value.getAsJsonObject().get(field)) : Optional.empty();
        } catch (IOException e) {
            Logger.global.logDebug("RenderStatus failed: " + e.getMessage());
            return Optional.empty();
        }
    }

    private boolean callBool(JsonObject header) {
        try {
            return backend.call(header).bool(true);
        } catch (IOException e) {
            Logger.global.logWarning(Msg.string(header, "t") + " failed: " + e.getMessage());
            return false;
        }
    }

}
