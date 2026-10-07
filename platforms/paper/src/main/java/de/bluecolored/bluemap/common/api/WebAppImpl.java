package de.bluecolored.bluemap.common.api;

import bluemaprs.paper.api.ShimBackend;
import de.bluecolored.bluemap.api.WebApp;
import de.bluecolored.bluemap.core.logger.Logger;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;
import java.util.Objects;
import java.util.UUID;
import java.util.stream.Stream;

public class WebAppImpl implements WebApp {

    private final ShimBackend backend;

    WebAppImpl(ShimBackend backend) {
        this.backend = backend;
    }

    @Override
    public Path getWebRoot() {
        return backend.webroot();
    }

    @Override
    public void setPlayerVisibility(UUID player, boolean visible) {
        backend.setPlayerVisibility(Objects.requireNonNull(player, "player"), visible);
    }

    @Override
    public boolean getPlayerVisibility(UUID player) {
        return backend.getPlayerVisibility(Objects.requireNonNull(player, "player"));
    }

    @Override
    public void registerScript(String url) {
        Logger.global.logDebug("Registering script from API: " + url);
        backend.registerScript(Objects.requireNonNull(url, "url"));
    }

    @Override
    public void registerStyle(String url) {
        Logger.global.logDebug("Registering style from API: " + url);
        backend.registerStyle(Objects.requireNonNull(url, "url"));
    }

    @Override
    @Deprecated(forRemoval = true)
    @SuppressWarnings("removal")
    public String createImage(BufferedImage image, String path) throws IOException {
        path = path.replaceAll("[^a-zA-Z0-9_.\\-/]", "_");

        Path webRoot = getWebRoot().toAbsolutePath();
        String separator = webRoot.getFileSystem().getSeparator();

        Path imageRootFolder = webRoot.resolve("data").resolve("images");
        Path imagePath = imageRootFolder.resolve(path.replace("/", separator) + ".png").toAbsolutePath();

        Files.createDirectories(imagePath.getParent());
        Files.deleteIfExists(imagePath);
        Files.createFile(imagePath);

        if (!ImageIO.write(image, "png", imagePath.toFile()))
            throw new IOException("The format 'png' is not supported!");

        return webRoot.relativize(imagePath).toString().replace(separator, "/");
    }

    @Override
    @Deprecated(forRemoval = true)
    @SuppressWarnings("removal")
    public Map<String, String> availableImages() throws IOException {
        Path webRoot = getWebRoot().toAbsolutePath();
        String separator = webRoot.getFileSystem().getSeparator();

        Path imageRootPath = webRoot.resolve("data").resolve("images").toAbsolutePath();

        Map<String, String> availableImagesMap = new HashMap<>();

        if (Files.exists(imageRootPath)) {
            try (Stream<Path> fileStream = Files.walk(imageRootPath)) {
                fileStream
                        .filter(p -> !Files.isDirectory(p))
                        .filter(p -> p.getFileName().toString().endsWith(".png"))
                        .map(Path::toAbsolutePath)
                        .forEach(p -> {
                            try {
                                String key = imageRootPath.relativize(p).toString();
                                key = key.substring(0, key.length() - 4).replace(separator, "/");
                                String value = webRoot.relativize(p).toString().replace(separator, "/");
                                availableImagesMap.put(key, value);
                            } catch (IllegalArgumentException ignore) {
                            }
                        });
            }
        }

        return availableImagesMap;
    }

}
