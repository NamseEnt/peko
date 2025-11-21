![](image.png)

This project was inspired by Cloudflare Workers.

Peko is a FaaS platform powered by [wasmtime](https://github.com/bytecodealliance/wasmtime) that executes users' Wasm.

## Features

- Any program that can be built into Wasm (WASI 0.2 - Component Model) can run on Peko.
- You can easily run the server in a local development environment using the CLI.
- You can run the Peko platform on various cloud providers using Adapters.
- Instead of managing the platform yourself, you can conveniently use Peko Cloud, a managed service.

## Supported Languages

- Rust
- JavaScript, TypeScript (without Node.js compatibility)

## Example Applications

- Server Side Rendering (SSR)
- Image Resizing
- Rest API Server

## Supported Cloud Providers

- Amazon Web Service (AWS)
- Oracle Cloud Infrastructure (OCI)

## Supported CDN Providers

- Cloudflare

## Supported Code Providers

- File System (Including NFS like AWS EFS)

## Internal Implementation

- Monolith architecture
- Load Balancing and Auto Scaling is provided by the cloud provider.
- When instance started, it runs Cloud Provider's Instance Discovery API to bootstrap, then use [membership](https://github.com/al8n/memberlist.git) to form a cluster.
- On user request;
  1. Instance finds [two](https://www.eecs.harvard.edu/~michaelm/postscripts/handbook2001.pdf) warmed up instances and forwards the request to less loaded instance.
  2. If forwarding is rejected, instance trys one more time.
  3. When instance rejected again or No warmed up instances, instance trys to find proper instance to run the request with cold-start.
     - Instance can start on itself if it has enough resources.
  4. If all instance is loaded, instance returns 503 Service Unavailable. And this should not happen under normal conditions. Must monitor this condition and alert to developers.
- Execution on Instnace;
  1. If instance doesn't have request's module, instance downloads module from Database.
  2. Or instance checks module's last modified time and downloads if it's updated.
  3. Instance instantiates module and runs it.
  4. Instance keeps wasm instance in memory cache for warm-start.

# License

agpl-3.0
