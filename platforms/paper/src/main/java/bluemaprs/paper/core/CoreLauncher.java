package bluemaprs.paper.core;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;
import java.util.Optional;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.logging.Logger;

/** Spawning the core and cleaning up after a previous one. */
final class CoreLauncher {

    private CoreLauncher() {}

    /** Two cores on one map corrupt render state: kill a live core left behind in {@code <data>/.core.pid}. */
    static void killStale(Path dataFolder, Logger log) {
        long pid;
        try {
            pid = Long.parseLong(Files.readString(dataFolder.resolve(".core.pid")).trim());
        } catch (IOException | NumberFormatException e) {
            return;
        }
        if (pid == ProcessHandle.current().pid()) return;
        Optional<ProcessHandle> handle = ProcessHandle.of(pid).filter(ProcessHandle::isAlive);
        if (handle.isEmpty()) return;
        String command = handle.get().info().command().orElse("");
        if (!command.toLowerCase(Locale.ROOT).contains("bluemap")) return;

        log.warning("Killing a leftover BlueMap core (pid " + pid + ", " + command + ")");
        handle.get().destroyForcibly();
        try {
            handle.get().onExit().get(10, TimeUnit.SECONDS);
        } catch (TimeoutException | ExecutionException e) {
            log.warning("The leftover BlueMap core (pid " + pid + ") did not exit within 10 s");
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    /** cwd = the server folder: BlueMap configs use paths relative to it. */
    static Process spawn(Path binary) throws IOException {
        return new ProcessBuilder(binary.toString(), "--plugin-ipc",
                "--parent-pid", Long.toString(ProcessHandle.current().pid()))
                .directory(Path.of("").toAbsolutePath().toFile())
                .start();
    }

    static void kill(Process process) {
        process.descendants().forEach(ProcessHandle::destroyForcibly);
        process.destroyForcibly();
        try {
            process.waitFor(5, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

}
