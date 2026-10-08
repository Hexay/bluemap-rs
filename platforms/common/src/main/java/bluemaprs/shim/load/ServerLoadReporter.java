package bluemaprs.shim.load;

import bluemaprs.shim.ipc.CoreLink;
import bluemaprs.shim.ipc.Frame;
import bluemaprs.shim.ipc.Msg;
import com.google.gson.JsonObject;
import org.slf4j.Logger;

import java.util.List;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.function.DoubleSupplier;

/** Sends the server's average tick time once a second ({@code ServerLoad}); the core pauses rendering on lag. */
public final class ServerLoadReporter {

    private final CoreLink link;
    private final DoubleSupplier averageTickMillis;
    private final Logger log;
    private final ScheduledExecutorService sender = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread thread = new Thread(r, "BlueMap-ServerLoad");
        thread.setDaemon(true);
        return thread;
    });

    public ServerLoadReporter(CoreLink link, DoubleSupplier averageTickMillis, Logger log) {
        this.link = link;
        this.averageTickMillis = averageTickMillis;
        this.log = log;
    }

    public void start() {
        sender.scheduleAtFixedRate(this::send, 1, 1, TimeUnit.SECONDS);
    }

    public void stop() {
        sender.shutdownNow();
    }

    private void send() {
        if (!link.connected()) return;
        try {
            double mspt = averageTickMillis.getAsDouble();
            if (!(mspt > 0) || Double.isInfinite(mspt)) return;
            JsonObject header = Msg.of("ServerLoad");
            header.addProperty("mspt", mspt);
            link.sendLatest("load", List.of(new Frame(header)));
        } catch (RuntimeException e) {
            log.debug("ServerLoad not sent: {}", e.toString());
        }
    }

}
