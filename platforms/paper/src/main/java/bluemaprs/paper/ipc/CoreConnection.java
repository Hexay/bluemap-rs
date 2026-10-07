package bluemaprs.paper.ipc;

import com.google.gson.JsonObject;

import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;
import java.util.logging.Level;
import java.util.logging.Logger;

/** One running core process: frame reader (stdout), single writer (stdin), stderr pump, pending requests. */
public final class CoreConnection {

    public static final String DOWN = "BlueMap core is not running";

    private static final int QUEUE_CAPACITY = 1024;
    private static final long QUEUE_TIMEOUT_MS = 5000;
    private static final long REQUEST_EXPIRY_S = 60;

    public record Exit(int code, boolean bye) {}

    private final Process process;
    private final Consumer<Frame> inbound;
    private final Logger log;
    private final OutboundQueue queue = new OutboundQueue(QUEUE_CAPACITY);
    private final Map<Long, CompletableFuture<Reply>> pending = new ConcurrentHashMap<>();
    private final CompletableFuture<Exit> exit = new CompletableFuture<>();
    private volatile boolean down, bye;

    public CoreConnection(Process process, Consumer<Frame> inbound, Logger log) {
        this.process = process;
        this.inbound = inbound;
        this.log = log;
    }

    public void start() {
        daemon("BlueMap-IPC-Reader", this::readLoop);
        daemon("BlueMap-IPC-Writer", this::writeLoop);
        daemon("BlueMap-Core-Stderr", this::pumpStderr);
    }

    public Process process() {
        return process;
    }

    /** Completes once the process exited and the reader is done. */
    public CompletableFuture<Exit> exit() {
        return exit;
    }

    public void send(Frame frame) throws IOException {
        if (down) throw new IOException(DOWN);
        queue.put(List.of(frame), QUEUE_TIMEOUT_MS);
    }

    public void sendLatest(String key, List<Frame> frames) {
        if (!down) queue.putLatest(key, frames);
    }

    public CompletableFuture<Reply> request(long id, JsonObject header, byte[] body) {
        header.addProperty("id", id);
        CompletableFuture<Reply> reply = new CompletableFuture<>();
        pending.put(id, reply);
        reply.orTimeout(REQUEST_EXPIRY_S, TimeUnit.SECONDS).whenComplete((r, e) -> pending.remove(id));
        try {
            send(new Frame(header, body));
        } catch (IOException e) {
            reply.completeExceptionally(e);
        }
        return reply;
    }

    /** Lets the writer drain the queue, then closes the core's stdin (EOF = stop). */
    public void closeInput() {
        queue.close();
    }

    private void readLoop() {
        try (InputStream in = new BufferedInputStream(process.getInputStream(), 1 << 16)) {
            Frame frame;
            while ((frame = FrameCodec.read(in)) != null) {
                switch (frame.type()) {
                    case "Reply" -> {
                        CompletableFuture<Reply> reply = pending.remove(frame.id());
                        if (reply != null) reply.complete(Reply.of(frame));
                    }
                    case "Bye" -> bye = true;
                    default -> dispatch(frame);
                }
            }
        } catch (IOException e) {
            if (!bye) log.warning("IPC stream from the BlueMap core broke: " + e.getMessage());
            process.destroyForcibly();
        } finally {
            down = true;
            queue.close();
            IOException gone = new IOException(DOWN);
            pending.values().forEach(r -> r.completeExceptionally(gone));
            pending.clear();
            exit.complete(new Exit(awaitExit(), bye));
        }
    }

    private void dispatch(Frame frame) {
        try {
            inbound.accept(frame);
        } catch (RuntimeException e) {
            log.log(Level.SEVERE, "Failed to handle IPC message " + frame.type(), e);
        }
    }

    private int awaitExit() {
        try {
            if (!process.waitFor(10, TimeUnit.SECONDS)) {
                process.destroyForcibly();
                process.waitFor(5, TimeUnit.SECONDS);
            }
            return process.isAlive() ? -1 : process.exitValue();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return -1;
        }
    }

    private void writeLoop() {
        try (OutputStream out = new BufferedOutputStream(process.getOutputStream(), 1 << 16)) {
            List<Frame> frames;
            while ((frames = queue.take()) != null) {
                for (Frame frame : frames) FrameCodec.write(out, frame);
                out.flush();
            }
        } catch (IOException e) {
            if (!down) log.warning("IPC stream to the BlueMap core broke: " + e.getMessage());
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        } finally {
            down = true;
            queue.close();
        }
    }

    private void pumpStderr() {
        try (BufferedReader err = new BufferedReader(
                new InputStreamReader(process.getErrorStream(), StandardCharsets.UTF_8))) {
            String line;
            while ((line = err.readLine()) != null) log.warning(line);
        } catch (IOException ignored) {
            // process gone
        }
    }

    private static void daemon(String name, Runnable task) {
        Thread thread = new Thread(task, name);
        thread.setDaemon(true);
        thread.start();
    }

}
