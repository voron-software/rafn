cmake_minimum_required(VERSION 3.14)
project(fibonacci_benchmark)

set(CMAKE_CXX_STANDARD 17)

include(FetchContent)

FetchContent_Declare(
    googlebenchmark
    GIT_REPOSITORY https://github.com/google/benchmark.git
    GIT_TAG v1.8.3
)

# Disable building tests for the benchmark library
set(BENCHMARK_ENABLE_TESTING OFF CACHE BOOL "" FORCE)
set(BENCHMARK_ENABLE_INSTALL OFF CACHE BOOL "" FORCE)

FetchContent_MakeAvailable(googlebenchmark)

add_executable(fibonacci_benchmark fibonacci_benchmark.cpp)
target_link_libraries(fibonacci_benchmark benchmark::benchmark benchmark::benchmark_main)
