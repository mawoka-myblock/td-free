#include "Adafruit_TCS34725.h"
#include "SPI.h"
#include <Wire.h>
#include <Arduino.h>
#include <WiFi.h>
#include <DNSServer.h>
#include <WebServer.h>
// put function declarations here:
Adafruit_TCS34725 tcs = Adafruit_TCS34725(TCS34725_INTEGRATIONTIME_300MS, TCS34725_GAIN_1X);
extern const uint8_t index_html_start[] asm("_binary_src_html_index_html_start");
extern const uint8_t index_html_end[] asm("_binary_src_html_index_html_end");

DNSServer dnsServer;
WebServer server(80);

float max_value;
float max_clear_value;
IPAddress apIP(192, 168, 0, 1);
IPAddress netMsk(255, 255, 255, 0);
float final_td_clear;

void replaceTemplateWithData(uint8_t *html, size_t htmlSize, const char *templateTag, float replacement)
{
  // Convert the binary data to a String
  String htmlString;
  for (size_t i = 0; i < htmlSize && html[i] != 0; i++)
  {
    htmlString += char(html[i]);
  }

  // Find and replace the template
  int pos = htmlString.indexOf(templateTag);
  if (pos != -1)
  {
    // Convert the float to a string with 2 decimal places
    char floatStr[10];                    // Adjust the size as needed
    dtostrf(replacement, 4, 2, floatStr); // 4 is the minimum width, 2 is the number of digits after the decimal point

    // Replace the template with the formatted float string
    htmlString.replace(templateTag, floatStr);

    // Update the non-const array with the modified String
    size_t newHtmlSize = htmlString.length();
    uint8_t *modifiedHtml = new uint8_t[newHtmlSize + 1]; // +1 for null terminator
    for (size_t i = 0; i < newHtmlSize; i++)
    {
      modifiedHtml[i] = static_cast<uint8_t>(htmlString.charAt(i));
    }
    modifiedHtml[newHtmlSize] = 0; // Null-terminate the modified array

    // Copy the modified content back to the original array
    memcpy(html, modifiedHtml, newHtmlSize + 1);

    // Release the memory allocated for the modified array
    delete[] modifiedHtml;
  }
}

void handleRoot()
{
  size_t htmlSize = 0;
  while (index_html_start[htmlSize] != 0)
  {
    htmlSize++;
  }

  // Create a non-const array for modification
  uint8_t *html = new uint8_t[htmlSize + 1]; // +1 for null terminator
  memcpy(html, index_html_start, htmlSize + 1);
  const char *templateTag = "{{value}}";
  float data = final_td_clear;
  replaceTemplateWithData(html, htmlSize, templateTag, data);
  server.send(200, "text/html", reinterpret_cast<const char *>(html));
  delete[] html;
}

void handleNotFound()
{
  server.sendHeader("Location", "/");
  server.send(302, "text/plain", "redirect to captive portal");
}

void setup(void)
{
  Serial.begin(9600);
  Serial.println("Boot ok!");

  WiFi.mode(WIFI_AP);
  WiFi.softAPConfig(apIP, apIP, netMsk);
  WiFi.softAP("Td-Test");
  dnsServer.start(53, "*", apIP);

  // serve a simple root page
  server.on("/", handleRoot);

  // serve portal page

  server.onNotFound(handleNotFound);
  server.begin();

  float max_reading[4];
  float max_clear[4];
  size_t size = 16;
  for (int i = 0; i < 4; i++)
  {
    uint16_t high_r, high_g, high_b, high_c;
    tcs.getRawData(&high_r, &high_g, &high_b, &high_c);
    max_reading[i] = (high_r + high_g + high_b + high_c);
    max_clear[i] = high_c;
    delay(500);
  }
  float sum = 0;
  float sum_c = 0;
  for (int i = 0; i < 4; ++i)
  {
    sum += max_reading[i];
    sum_c += max_clear[i];
  }
  max_value = sum / 4;
  max_clear_value = sum_c / 4;
  // Now we're ready to get readings!
}

void loop(void)
{
  Serial.println("LOOP");
  uint16_t r, g, b, c, colorTemp;
  float red, green, blue;

  tcs.getRawData(&r, &g, &b, &c);
  tcs.getRGB(&red, &green, &blue);
  float transmission = (r + g + b + c); // Adjust the combination method as needed
  float td_percent = (transmission / max_value) * 100;
  float final_td = 0.812 * td_percent - 0.834;
  float clear_percent = c / max_clear_value;

  final_td_clear = 120.45848313466772 * clear_percent + 1.2720622896060854;
  dnsServer.processNextRequest();
  server.handleClient();
  delay(100);
}