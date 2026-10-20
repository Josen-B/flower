cmake_minimum_required(VERSION 3.24)

project(FlowerIme VERSION 0.1.0 LANGUAGES CXX)

if(NOT WIN32)
  message(FATAL_ERROR "Flower is a Windows-only TSF input method.")
endif()

option(FLOWER_BUILD_TESTS "Build Flower's focused unit tests" ON)

add_library(flower_engine STATIC
  src/engine/rime_engine.cpp
)

target_compile_features(flower_engine PUBLIC cxx_std_17)
target_include_directories(flower_engine
  PUBLIC src
  PRIVATE
    third_party/librime/src
    third_party/librime/include
)
target_compile_definitions(flower_engine
  PRIVATE
    UNICODE
    _UNICODE
    WIN32_LEAN_AND_MEAN
    NOMINMAX
    _WIN32_WINNT=0x0A00
)

if(MSVC)
  target_compile_options(flower_engine PRIVATE /W4 /permissive- /EHsc)
else()
  target_compile_options(flower_engine PRIVATE -Wall -Wextra -Wpedantic)
endif()

if(FLOWER_BUILD_TESTS)
  enable_testing()
  add_executable(flower_engine_tests
    tests/engine_key_tests.cpp
  )
  target_include_directories(flower_engine_tests
    PRIVATE third_party/librime/include
  )
  target_link_libraries(flower_engine_tests PRIVATE flower_engine)
  target_compile_features(flower_engine_tests PRIVATE cxx_std_17)
  add_test(NAME flower_engine_tests COMMAND flower_engine_tests)
endif()

install(TARGETS flower_engine
  ARCHIVE DESTINATION lib
  LIBRARY DESTINATION lib
  RUNTIME DESTINATION bin
)
install(FILES
  data/rime/default.yaml
  data/rime/flower_pinyin.schema.yaml
  third_party/rime-pinyin-simp/pinyin_simp.dict.yaml
  DESTINATION share/flower/rime
)
install(FILES
  third_party/librime/LICENSE
  DESTINATION share/flower/licenses/librime
)
install(FILES
  third_party/rime-pinyin-simp/LICENSE
  DESTINATION share/flower/licenses/rime-pinyin-simp
)
