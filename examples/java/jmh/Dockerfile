FROM maven:3.9-eclipse-temurin-21 AS builder

WORKDIR /build

COPY pom.xml .
COPY src ./src

RUN mvn clean package

FROM eclipse-temurin:21-jre

WORKDIR /app

COPY --from=builder /build/target/benchmarks.jar .

RUN mkdir -p /results

CMD ["java", "-jar", "benchmarks.jar", "-rf", "json", "-rff", "/results/jmh-result.json"]
