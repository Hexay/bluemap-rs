package de.bluecolored.bluemap.core.logger;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.logging.FileHandler;
import java.util.logging.Level;
import java.util.stream.StreamSupport;

/** Facade of upstream's core logger (same public surface) for addons like BMUtils; the plugin routes it to JUL. */
public abstract class Logger implements AutoCloseable {

    @SuppressWarnings("StaticInitializerReferencesSubClass")
    public static final MultiLogger global = new MultiLogger(stdOut());
    static {
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            try {
                global.close();
            } catch (Exception ignore) {}
        }));
    }

    public void logError(Throwable throwable) {
        logError(throwable.getMessage(), throwable);
    }

    public abstract void logError(String message, Throwable throwable);

    public abstract void logWarning(String message);

    public abstract void logInfo(String message);

    public abstract void logDebug(String message);

    public abstract void noFloodError(String key, String message, Throwable throwable);

    public abstract void noFloodWarning(String key, String message);

    public abstract void noFloodInfo(String key, String message);

    public abstract void noFloodDebug(String key, String message);

    public void noFloodError(Throwable throwable) {
        noFloodError(throwable.getMessage(), throwable);
    }

    public void noFloodError(String message, Throwable throwable) {
        noFloodError(message, message, throwable);
    }

    public void noFloodWarning(String message) {
        noFloodWarning(message, message);
    }

    public void noFloodInfo(String message) {
        noFloodInfo(message, message);
    }

    public void noFloodDebug(String message) {
        noFloodDebug(message, message);
    }

    @Override
    public void close() throws Exception {}

    public abstract void clearNoFloodLog();

    public abstract void removeNoFloodKey(String key);

    public void removeNoFloodMessage(String message) {
        removeNoFloodKey(message);
    }

    public static Logger stdOut() {
        return new PrintStreamLogger(System.out, System.err);
    }

    public static Logger stdOut(boolean debug) {
        return new PrintStreamLogger(System.out, System.err, debug);
    }

    public static Logger file(Path path) throws IOException {
        return file(path, null);
    }

    public static Logger file(Path path, boolean append) throws IOException {
        return file(path, null, append);
    }

    public static Logger file(Path path, String format) throws IOException {
        return file(path, format, true);
    }

    public static Logger file(Path path, String format, boolean append) throws IOException {
        Files.createDirectories(path.getParent());

        FileHandler fileHandler = new FileHandler(path.toString(), append);
        fileHandler.setFormatter(format == null ? new LogFormatter() : new LogFormatter(format));

        java.util.logging.Logger javaLogger = java.util.logging.Logger.getAnonymousLogger();
        javaLogger.setLevel(Level.ALL);
        javaLogger.setUseParentHandlers(false);
        javaLogger.addHandler(fileHandler);

        return new JavaLogger(javaLogger);
    }

    public static Logger combine(Iterable<Logger> logger) {
        return combine(StreamSupport.stream(logger.spliterator(), false).toArray(Logger[]::new));
    }

    public static Logger combine(Logger... logger) {
        if (logger.length == 0) return new VoidLogger();
        if (logger.length == 1) return logger[0];
        return new MultiLogger(logger);
    }

}
