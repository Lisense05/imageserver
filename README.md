# imageserver
A bare bones image server

This project is considered basically done for its intended purpose

The apis are as follows

## GET
Gets the specific image

https://localhost/v1/images/image.png

Gets an embeded url through the image server and gives it to the client so only the server ip is revealed and not the client ip

https://localhost/v1/embed?url=https://somescetchywebsite.com/image.png

## POST
Uploads an image

https://localhost/v1/images

## Authentication

Image and audio uploads now require an `Authorization` request header with the same value as `api_key` in `config.toml`. Requests with a missing or invalid header return `401` immediately, before multipart body parsing.

Example request setup for `POST /v1/image`:
- Header: `Authorization: change-me`
- Body: `form-data` with a file field (for example `file`)

## Rate limiting

Upload endpoints are rate-limited per client IP using `rate_limit_max_requests` and `rate_limit_window_seconds` from `config.toml`. When the limit is exceeded, the server returns `429`.

