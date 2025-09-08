
# Async Control Flow in `wgpu` Example

This document outlines the asynchronous control flow for the `wgpu` example, highlighting the differences between the desktop and web platforms.

## Desktop Control Flow

```mermaid
graph TD
    A[run] --> B{EventLoop};
    B --> C[App::new];
    C --> D[ScriptEngineDesktop];
    B --> E{run_app};
    E --> F[resumed event];
    F --> G[Create Window];
    G --> H[block_on(load_javascript_file)];
    H --> I[block_on(State::new)];
    I --> J[call_demo_functions];
    E --> K{Event Loop};
    K --> L[RedrawRequested];
    L --> M[call update in JS];
    M --> N[render];
```

**Desktop Summary:**

- The application starts with the `run` function.
- It uses `pollster::block_on` to handle async operations in a synchronous manner.
- JavaScript files are loaded and the `State` is initialized within the `resumed` event handler.
- The application then enters the main event loop.

## Web Control Flow

```mermaid
graph TD
    subgraph Main Thread
        A[run_web] --> B[run];
        B --> C{EventLoop};
        C --> D[App::new];
        D --> E[ScriptEngineWeb];
        D --> F[EventLoopProxy];
        C --> G{run_app};
        G --> H[resumed event];
        H --> I[Create Window];
    end

    subgraph "Web Worker / JS"
        J[spawn_local(load_javascript_file)];
        K[spawn_local(State::new)];
    end

    subgraph Main Thread
        H --> J;
        H --> K;
        K -- send_event --> L[user_event];
        L --> M[Set State];
        M --> N[call_demo_functions];
        G --> O{Event Loop};
        O --> P[RedrawRequested];
        P --> Q[call update in JS];
        Q --> R[render];
    end
```

**Web Summary:**

- The application starts with the `run_web` function, which is the `wasm-bindgen` entry point.
- It uses `wasm_bindgen_futures::spawn_local` to handle async operations without blocking the main thread.
- JavaScript loading and `State` creation happen concurrently.
- Once the `State` is initialized, it is sent back to the main thread via a `user_event`.
- The `user_event` handler sets the state and calls the initial JavaScript functions.
- The application then enters the main event loop.
