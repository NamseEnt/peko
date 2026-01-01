import { core } from "ext:core/mod.js";

import * as webidl from "ext:deno_webidl/00_webidl.js";

// Import run.js to ensure it's evaluated during snapshot creation
import { runHandler } from "ext:bootstrap/run.js";

// Expose runHandler to globalThis for runtime execution
Object.defineProperty(globalThis, "__ski_runHandler", {
  value: runHandler,
  enumerable: false,
  configurable: false,
  writable: false,
});

import "ext:deno_web/00_infra.js";
import * as url from "ext:deno_web/00_url.js";
import * as console from "ext:deno_web/01_console.js";
import { DOMException } from "ext:deno_web/01_dom_exception.js";
import "ext:deno_web/01_mimesniff.js";
import * as urlPattern from "ext:deno_web/01_urlpattern.js";
import * as event from "ext:deno_web/02_event.js";
import * as structuredCloneModule from "ext:deno_web/02_structured_clone.js";
import * as timers from "ext:deno_web/02_timers.js";
import * as abortSignal from "ext:deno_web/03_abort_signal.js";
import "ext:deno_web/04_global_interfaces.js";
import * as base64 from "ext:deno_web/05_base64.js";
import * as streams from "ext:deno_web/06_streams.js";
import * as encoding from "ext:deno_web/08_text_encoding.js";
import * as file from "ext:deno_web/09_file.js";
import * as messagePort from "ext:deno_web/13_message_port.js";
import * as compression from "ext:deno_web/14_compression.js";
import * as performance from "ext:deno_web/15_performance.js";

import * as headers from "ext:deno_fetch/20_headers.js";
import * as formData from "ext:deno_fetch/21_formdata.js";
import * as request from "ext:deno_fetch/23_request.js";
import * as response from "ext:deno_fetch/23_response.js";
import * as fetch from "ext:deno_fetch/26_fetch.js";

Object.defineProperty(globalThis, "fetch", {
  value: fetch.fetch,
  enumerable: true,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "Request", {
  value: request.Request,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "Response", {
  value: response.Response,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "Headers", {
  value: headers.Headers,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "FormData", {
  value: formData.FormData,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "console", {
  value: new console.Console((msg, level) => core.print(msg, level > 1)),
  enumerable: false,
  configurable: true,
  writable: true,
});

// URL APIs
Object.defineProperty(globalThis, "URL", {
  value: url.URL,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "URLSearchParams", {
  value: url.URLSearchParams,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "URLPattern", {
  value: urlPattern.URLPattern,
  enumerable: false,
  configurable: true,
  writable: true,
});

// DOM Exception
Object.defineProperty(globalThis, "DOMException", {
  value: DOMException,
  enumerable: false,
  configurable: true,
  writable: true,
});

// Event APIs
Object.defineProperty(globalThis, "Event", {
  value: event.Event,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "EventTarget", {
  value: event.EventTarget,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "CustomEvent", {
  value: event.CustomEvent,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "MessageEvent", {
  value: event.MessageEvent,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "CloseEvent", {
  value: event.CloseEvent,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "ErrorEvent", {
  value: event.ErrorEvent,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "ProgressEvent", {
  value: event.ProgressEvent,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "PromiseRejectionEvent", {
  value: event.PromiseRejectionEvent,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "reportError", {
  value: event.reportError,
  enumerable: true,
  configurable: true,
  writable: true,
});

// Timer APIs
Object.defineProperty(globalThis, "setTimeout", {
  value: timers.setTimeout,
  enumerable: true,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "setInterval", {
  value: timers.setInterval,
  enumerable: true,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "clearTimeout", {
  value: timers.clearTimeout,
  enumerable: true,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "clearInterval", {
  value: timers.clearInterval,
  enumerable: true,
  configurable: true,
  writable: true,
});

// AbortController and AbortSignal
Object.defineProperty(globalThis, "AbortController", {
  value: abortSignal.AbortController,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "AbortSignal", {
  value: abortSignal.AbortSignal,
  enumerable: false,
  configurable: true,
  writable: true,
});

// Base64 APIs
Object.defineProperty(globalThis, "atob", {
  value: base64.atob,
  enumerable: true,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "btoa", {
  value: base64.btoa,
  enumerable: true,
  configurable: true,
  writable: true,
});

// Stream APIs
Object.defineProperty(globalThis, "ReadableStream", {
  value: streams.ReadableStream,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "ReadableStreamDefaultReader", {
  value: streams.ReadableStreamDefaultReader,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "ReadableStreamBYOBReader", {
  value: streams.ReadableStreamBYOBReader,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "ReadableStreamBYOBRequest", {
  value: streams.ReadableStreamBYOBRequest,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "ReadableByteStreamController", {
  value: streams.ReadableByteStreamController,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "ReadableStreamDefaultController", {
  value: streams.ReadableStreamDefaultController,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "WritableStream", {
  value: streams.WritableStream,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "WritableStreamDefaultWriter", {
  value: streams.WritableStreamDefaultWriter,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "WritableStreamDefaultController", {
  value: streams.WritableStreamDefaultController,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "TransformStream", {
  value: streams.TransformStream,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "TransformStreamDefaultController", {
  value: streams.TransformStreamDefaultController,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "ByteLengthQueuingStrategy", {
  value: streams.ByteLengthQueuingStrategy,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "CountQueuingStrategy", {
  value: streams.CountQueuingStrategy,
  enumerable: false,
  configurable: true,
  writable: true,
});

// Text Encoding APIs
Object.defineProperty(globalThis, "TextEncoder", {
  value: encoding.TextEncoder,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "TextDecoder", {
  value: encoding.TextDecoder,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "TextEncoderStream", {
  value: encoding.TextEncoderStream,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "TextDecoderStream", {
  value: encoding.TextDecoderStream,
  enumerable: false,
  configurable: true,
  writable: true,
});

// File APIs
Object.defineProperty(globalThis, "Blob", {
  value: file.Blob,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "File", {
  value: file.File,
  enumerable: false,
  configurable: true,
  writable: true,
});

// Compression APIs
Object.defineProperty(globalThis, "CompressionStream", {
  value: compression.CompressionStream,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "DecompressionStream", {
  value: compression.DecompressionStream,
  enumerable: false,
  configurable: true,
  writable: true,
});

// MessageChannel and MessagePort
Object.defineProperty(globalThis, "MessageChannel", {
  value: messagePort.MessageChannel,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "MessagePort", {
  value: messagePort.MessagePort,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "structuredClone", {
  value: messagePort.structuredClone,
  enumerable: true,
  configurable: true,
  writable: true,
});

// Performance APIs
Object.defineProperty(globalThis, "Performance", {
  value: performance.Performance,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "PerformanceEntry", {
  value: performance.PerformanceEntry,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "PerformanceMark", {
  value: performance.PerformanceMark,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "PerformanceMeasure", {
  value: performance.PerformanceMeasure,
  enumerable: false,
  configurable: true,
  writable: true,
});

Object.defineProperty(globalThis, "performance", {
  value: performance.performance,
  enumerable: true,
  configurable: true,
  writable: true,
});
