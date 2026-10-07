package bluemaprs.shim.core;

import bluemaprs.shim.api.ShimBackend;
import bluemaprs.shim.commands.CommandBridge;
import bluemaprs.shim.ipc.CoreLink;
import bluemaprs.shim.ipc.Frame;
import bluemaprs.shim.ipc.Msg;
import bluemaprs.shim.ipc.Proto.StateInfo;
import bluemaprs.shim.markers.MarkerPusher;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonPrimitive;
import org.slf4j.Logger;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.function.Function;

/** Core → shim messages outside the lifecycle (which {@link CoreSupervisor} handles); runs on the IPC reader. */
public final class CoreDispatcher {

    private final Logger log;
    private final CoreLink link;
    private final ShimBackend backend;
    private final MarkerPusher markers;
    private final CommandBridge<?> command;
    private final Function<String, CompletableFuture<Boolean>> saveWorld;

    /** @param saveWorld {@code WorldInfo.id} or null for all → whether the world(s) were saved */
    public CoreDispatcher(Logger log, CoreLink link, ShimBackend backend, MarkerPusher markers,
                          CommandBridge<?> command, Function<String, CompletableFuture<Boolean>> saveWorld) {
        this.log = log;
        this.link = link;
        this.backend = backend;
        this.markers = markers;
        this.command = command;
        this.saveWorld = saveWorld;
    }

    void handle(Frame frame) {
        JsonObject h = frame.header();
        switch (frame.type()) {
            case "Log" -> log(h);
            case "StateChanged" -> backend.updateState(Msg.parse(h, StateInfo.class));
            case "CommandOutput" -> command.output(frame.id(), h.get("component"));
            case "CommandDone" -> command.done(frame.id());
            case "SaveWorld" -> saveWorld(frame.id(), Msg.string(h, "world"));
            case "MarkerDemand" -> markers.setDemand(strings(h.get("maps")));
            default -> log.debug("Ignoring unknown IPC message {}", frame.type());
        }
    }

    void coreGone() {
        command.abortAll();
    }

    private void log(JsonObject h) {
        String msg = Msg.string(h, "msg");
        String trace = Msg.string(h, "trace");
        if (trace != null && !trace.isEmpty()) msg = msg + "\n" + trace;
        switch (String.valueOf(Msg.string(h, "level"))) {
            case "debug" -> log.debug(msg);
            case "warning" -> log.warn(msg);
            case "error" -> log.error(msg);
            default -> log.info(msg);
        }
    }

    private void saveWorld(long id, String world) {
        CompletableFuture<Boolean> saved;
        try {
            saved = saveWorld.apply(world);
        } catch (RuntimeException e) {
            saved = CompletableFuture.failedFuture(e);
        }
        saved.whenComplete((ok, e) -> {
            JsonObject reply = e == null
                    ? Msg.reply(id, true, null, new JsonPrimitive(Boolean.TRUE.equals(ok)))
                    : Msg.reply(id, false, "saving failed: " + e, null);
            if (e != null) log.warn("Failed to save " + (world == null ? "the worlds" : world), e);
            try {
                link.send(new Frame(reply));
            } catch (IOException ignored) {
                // core gone; it no longer waits for this reply
            }
        });
    }

    private static List<String> strings(JsonElement array) {
        List<String> out = new ArrayList<>();
        if (array != null && array.isJsonArray()) array.getAsJsonArray().forEach(e -> out.add(e.getAsString()));
        return out;
    }

}
