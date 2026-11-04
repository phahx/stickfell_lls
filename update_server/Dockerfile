# The Stickfell update service. Built from the repo root (it needs keys/):
#   docker build -f update_server/Dockerfile .
FROM node:22-alpine AS build
WORKDIR /app/update_server
COPY update_server/package.json update_server/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY update_server/tsconfig.json ./
COPY update_server/src ./src
RUN npm run build

FROM node:22-alpine
ENV NODE_ENV=production
WORKDIR /app/update_server
COPY update_server/package.json ./
COPY --from=build /app/update_server/dist ./dist
COPY keys /app/keys
# (runs as root: Railway volumes are owned by root)
CMD ["node", "dist/server.js"]
