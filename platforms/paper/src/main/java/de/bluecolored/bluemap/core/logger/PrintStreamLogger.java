package de.bluecolored.bluemap.core.logger;

import java.io.PrintStream;
import java.time.Instant;
import java.time.ZoneId;
import java.time.ZonedDateTime;

public class PrintStreamLogger extends AbstractLogger {

    private final PrintStream out, err;

    boolean isDebug;

    public PrintStreamLogger(PrintStream out, PrintStream err) {
        this(out, err, false);
    }

    public PrintStreamLogger(PrintStream out, PrintStream err, boolean debug) {
        this.out = out;
        this.err = err;
        this.isDebug = debug;
    }

    public boolean isDebug() {
        return isDebug;
    }

    public void setDebug(boolean debug) {
        this.isDebug = debug;
    }

    @Override
    public void logError(String message, Throwable throwable) {
        log(err, "ERROR", message);
        if (throwable != null) throwable.printStackTrace(err);
    }

    @Override
    public void logWarning(String message) {
        log(out, "WARNING", message);
    }

    @Override
    public void logInfo(String message) {
        log(out, "INFO", message);
    }

    @Override
    public void logDebug(String message) {
        if (isDebug) log(out, "DEBUG", message);
    }

    @Override
    public void noFloodDebug(String key, String message) {
        if (isDebug) super.noFloodDebug(key, message);
    }

    @Override
    public void noFloodDebug(String message) {
        if (isDebug) super.noFloodDebug(message);
    }

    private void log(PrintStream stream, String level, String message) {
        ZonedDateTime zdt = ZonedDateTime.ofInstant(Instant.now(), ZoneId.systemDefault());
        stream.printf("[%1$tT %2$s] %3$s%n", zdt, level, message);
    }

}
