# Build context: repository root.
FROM python:3.12-slim
RUN pip install --no-cache-dir edge-tts fastapi uvicorn lameenc
WORKDIR /app
COPY docker/tts/server.py ./
ENV TTS_DATA=/data TTS_RATE=+0%
VOLUME /data
EXPOSE 8016
CMD ["uvicorn", "server:app", "--host", "0.0.0.0", "--port", "8016"]
