CFLAGS = -Wall -Werror -O3
LDFLAGS = -lstatsdclient
TARGET = statsd-sampleapp-c
SOURCES = main.c
OBJECTS = $(SOURCES:.c=.o)

.PHONY: all clean

all: $(TARGET)

clean:
	rm -f *.o $(TARGET)

$(TARGET): $(OBJECTS)
	$(CC) $^ $(LDFLAGS) -o $@

%.o: %.c
	$(CC) $(CFLAGS) -c $<
