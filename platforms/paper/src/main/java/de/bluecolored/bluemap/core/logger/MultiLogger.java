package de.bluecolored.bluemap.core.logger;

import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.locks.ReentrantReadWriteLock;
import java.util.function.Consumer;

public class MultiLogger extends AbstractLogger {

    private static final AtomicInteger LOGGER_INDEX = new AtomicInteger(0);

    private final ReentrantReadWriteLock lock = new ReentrantReadWriteLock();
    private final Map<String, Logger> logger;

    public MultiLogger(Logger... logger) {
        this.logger = new HashMap<>();
        for (Logger l : logger)
            put(l);
    }

    public void put(Logger logger) {
        put("anonymous-logger-" + LOGGER_INDEX.getAndIncrement(), () -> logger);
    }

    public void put(String name, LoggerSupplier loggerSupplier) {
        lock.writeLock().lock();
        try {
            remove(name);
            this.logger.put(name, loggerSupplier.get());
        } catch (Exception ex) {
            logError("Failed to close Logger!", ex);
        } finally {
            lock.writeLock().unlock();
        }
    }

    public void remove(String name) {
        lock.writeLock().lock();
        try {
            Logger removed = this.logger.remove(name);
            if (removed != null) removed.close();
        } catch (Exception ex) {
            logError("Failed to close Logger!", ex);
        } finally {
            lock.writeLock().unlock();
        }
    }

    public void clear() {
        lock.writeLock().lock();
        try {
            for (String name : this.logger.keySet().toArray(String[]::new))
                remove(name);
            this.logger.clear();
        } finally {
            lock.writeLock().unlock();
        }
    }

    @Override
    public void logError(String message, Throwable throwable) {
        forEach(l -> l.logError(message, throwable));
    }

    @Override
    public void logWarning(String message) {
        forEach(l -> l.logWarning(message));
    }

    @Override
    public void logInfo(String message) {
        forEach(l -> l.logInfo(message));
    }

    @Override
    public void logDebug(String message) {
        forEach(l -> l.logDebug(message));
    }

    @Override
    public void close() throws Exception {
        lock.readLock().lock();
        try {
            for (Logger l : logger.values())
                l.close();
        } finally {
            lock.readLock().unlock();
        }
    }

    private void forEach(Consumer<Logger> action) {
        lock.readLock().lock();
        try {
            logger.values().forEach(action);
        } finally {
            lock.readLock().unlock();
        }
    }

    @FunctionalInterface
    public interface LoggerSupplier {

        Logger get() throws Exception;

    }

}
